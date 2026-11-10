#!/usr/bin/env python3
"""Build and stage the installed Storage Settings app and its locked Electron runtime."""
import argparse
import json
import os
import plistlib
from pathlib import Path
import shutil
import subprocess
import sys



def sign_runtime(runtime, sign, entitlements):
    bundles = [p for p in runtime.rglob("*") if not p.is_symlink()
               and p.is_dir() and p.suffix in (".app", ".framework")] + [runtime]
    executables = set()
    for bundle in bundles:
        info = bundle / ("Contents/Info.plist" if bundle.suffix == ".app" else "Resources/Info.plist")
        with info.open("rb") as stream:
            metadata = plistlib.load(stream)
        # Electron helper bundles omit this optional key; macOS uses the bundle name.
        name = metadata.get("CFBundleExecutable", bundle.stem)
        executable = bundle / ("Contents/MacOS" if bundle.suffix == ".app" else "") / name
        if not executable.is_file():
            raise ValueError(f"Missing bundle executable: {executable}")
        executables.add(executable.resolve())
    magic = {b"\xfe\xed\xfa\xce", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca"}
    # Signing a bundle's principal executable can sign/validate the whole bundle.
    # Defer it to the bundle pass, after all nested code has been signed.
    for file in runtime.rglob("*"):
        if file.is_symlink() or not file.is_file() or file.resolve() in executables:
            continue
        with file.open("rb") as stream:
            is_code = stream.read(4) in magic
        if is_code:
            subprocess.run(sign + ["--entitlements", str(entitlements), str(file)], check=True)
    for bundle in sorted(bundles, key=lambda p: len(p.parts), reverse=True):
        subprocess.run(sign + ["--entitlements", str(entitlements), str(bundle)], check=True)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(runtime)], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--sign", help="macOS signing identity; '-' for ad hoc")
    parser.add_argument("--keychain", type=Path, help="job-private macOS signing keychain")
    args = parser.parse_args()
    if sys.platform not in ("darwin", "linux"):
        parser.error("Settings packaging supports macOS and Linux")
    root = Path(__file__).resolve().parents[4]
    frontend = root / "capture/desktop/ui/frontend"
    env = os.environ.copy()
    env.pop("ELECTRON_SKIP_BINARY_DOWNLOAD", None)
    subprocess.run(["npm", "ci", "--ignore-scripts", "--no-audit", "--no-fund"], cwd=frontend, env=env, check=True)
    subprocess.run(["node", "node_modules/electron/install.js"], cwd=frontend, env=env, check=True)
    subprocess.run(["npm", "run", "build"], cwd=frontend, env=env, check=True)
    destination = args.destination.absolute()
    if destination.is_symlink():
        parser.error("Refusing a symlink staging destination")
    if destination.exists():
        shutil.rmtree(destination)
    app = destination / "app"
    app.mkdir(parents=True)
    shutil.copytree(frontend / "electron", app / "electron")
    theme = app / "node_modules/@apparent/ui"
    (theme / "src").mkdir(parents=True)
    for name in ("window-theme.cjs", "tokens.css"):
        shutil.copy2(root / "shared/ui/src" / name, theme / "src" / name)
    (theme / "package.json").write_text(json.dumps({"exports": {"./window-theme": "./src/window-theme.cjs"}}) + "\n")
    shutil.copytree(frontend / "dist/assets", app / "dist/assets",
                    ignore=shutil.ignore_patterns("foundation-*"))
    shutil.copy2(frontend / "dist/settings.html", app / "dist/settings.html")
    shutil.copy2(frontend / "dist/onboarding.html", app / "dist/onboarding.html")
    for name in ("presentation", "hyprland", "account"):
        shutil.copy2(frontend / f"dist/onboarding-{name}.cjs", app / f"dist/onboarding-{name}.cjs")
    shutil.copytree(frontend / "dist/pill", app / "dist/pill")
    installer_ui = app / "electron/installer/ui"
    (installer_ui / "src/installer").mkdir(parents=True)
    shutil.copy2(frontend / "dist/src/installer/index.html", installer_ui / "src/installer/index.html")
    shutil.copytree(frontend / "dist/assets", installer_ui / "assets")
    # Authored hosts stay TypeScript; only generated CJS enters the signed package.
    subprocess.run(["node", "-e", """
const fs = require('node:fs'), path = require('node:path'), ts = require('typescript');
const destination = process.argv[1];
for (const name of ['main', 'preload']) {
  const file = path.join('electron/installer', name + '.cts');
  const result = ts.transpileModule(fs.readFileSync(file, 'utf8'), {
    fileName: file, reportDiagnostics: true,
    compilerOptions: {module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, esModuleInterop: true}
  });
  if (result.diagnostics?.length) throw Error('Installer host transpilation failed');
  fs.writeFileSync(path.join(destination, name + '.cjs'), result.outputText);
}
""", str(app / "electron/installer")], cwd=frontend, check=True)
    (app / "dist/src/controls").mkdir(parents=True)
    shutil.copy2(frontend / "dist/src/controls/index.html", app / "dist/src/controls/index.html")
    (app / "package.json").write_text(json.dumps({"name": "apparent-settings", "version": "0.0.0", "main": "electron/settings.cjs"}) + "\n")
    shutil.copytree(frontend / "node_modules/electron/dist", destination / "runtime", symlinks=True)
    if sys.platform == "darwin":
        if not args.sign:
            parser.error("macOS packaging requires an explicit signing identity")
        runtime = destination / "runtime/Electron.app"
        entitlements = Path(__file__).with_name("Settings.entitlements")
        sign = ["codesign", "--force", "--options", "runtime", "--sign", args.sign]
        if args.sign != "-":
            sign.append("--timestamp")
        if args.keychain:
            sign.extend(["--keychain", str(args.keychain)])
        sign_runtime(runtime, sign, entitlements)
    print(f"Staged Storage Settings: {destination}")


if __name__ == "__main__":
    main()
