# Checks a Windows build directory (scrawl.exe and transcribe.cpp's DLLs) for #58: no AVX-512
# outside the ggml CPU backends that are only loaded on a CPU that has it.
#
#   pwsh scripts/check-windows-cpu-tiers.ps1 target\x86_64-pc-windows-msvc\release

param([Parameter(Mandatory = $true)][string]$Dir)
$ErrorActionPreference = "Stop"

$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products * -property installationPath
$dumpbin = (Get-ChildItem "$vs\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" | Select-Object -First 1).FullName

# An EVEX prefix (0x62, which no other x86-64 instruction starts with, after at most a segment or
# address-size prefix) is AVX-512, whatever register width it uses. MSVC's auto-vectorizer adds a
# few on its own (vpmullq for 64-bit multiplies), behind its runtime check of the CRT's
# `__isa_available` (`cmp <level>,6` then `jl` around the AVX-512 loop): those are allowed.
function Avx512Lines([string]$file) {
  @(& $dumpbin /nologo /disasm $file |
    Select-String -Pattern '^\s+[0-9A-F]+: (6[4-7] )?62 [0-9A-F]{2} [0-9A-F]{2} [0-9A-F]{2} ' -CaseSensitive -Context 60, 2)
}

function Guarded($hit) {
  $before = @($hit.Context.PreContext)
  for ($i = 0; $i -lt $before.Count; $i++) {
    if ($before[$i] -notmatch '\scmp\s+\S.*,6\s*$') { continue }
    # The compiler may schedule a few unrelated instructions between the compare and the branch.
    for ($j = $i + 1; $j -lt [Math]::Min($i + 8, $before.Count); $j++) {
      if ($before[$j] -match '\sj(l|b)\s') { return $true }
      if ($before[$j] -match '\s(cmp|test)\s') { break }
    }
  }
  $false
}

$imports = & $dumpbin /nologo /dependents (Join-Path $Dir "scrawl.exe")
if (-not ($imports -match '^\s+transcribe\.dll\s*$')) {
  throw "scrawl.exe doesn't load transcribe.dll: ggml is linked in, compiled for the build machine's CPU"
}

# Loaded on every CPU, or on CPUs without AVX-512 (ggml scores each tier against CPUID).
$baseline = @("transcribe.dll", "ggml.dll", "ggml-base.dll",
  "ggml-cpu-x64.dll", "ggml-cpu-sse42.dll", "ggml-cpu-sandybridge.dll",
  "ggml-cpu-haswell.dll", "ggml-cpu-alderlake.dll")
$failed = $false
foreach ($name in $baseline) {
  $file = Join-Path $Dir $name
  if (-not (Test-Path $file)) { throw "missing $name in $Dir" }
  $all = Avx512Lines $file
  $hits = @($all | Where-Object { -not (Guarded $_) })
  if ($hits.Count -gt 0) {
    "$name has $($hits.Count) AVX-512 instructions without a CPU check, e.g.:"
    $hits | Select-Object -First 2 | ForEach-Object { $_.Context.PreContext; ">> $($_.Line)"; $_.Context.PostContext; "" }
    $failed = $true
  } else {
    "$name`: no AVX-512 ($($all.Count) behind MSVC's __isa_available check)"
  }
}

# The check has to be able to see AVX-512 at all.
$skx = Join-Path $Dir "ggml-cpu-skylakex.dll"
if (-not (Test-Path $skx)) { throw "missing ggml-cpu-skylakex.dll in $Dir" }
if ((Avx512Lines $skx).Count -eq 0) { throw "found no AVX-512 even in ggml-cpu-skylakex.dll: the check is broken" }

if ($failed) { throw "AVX-512 in code every CPU runs (#58)" }
