//! AppImage updates. The running inode remains in use; the new image starts on the next launch.
//! Never replace a debug build, a symlink, or somebody else's /opt installation.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::fd::AsRawFd;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const REPO: &str = "tryapparent/apparent";
static CHECK: Mutex<()> = Mutex::new(());

pub fn set_channel_in(path: &Path, channel: &str) -> Result<()> {
    ensure!(matches!(channel, "stable" | "main"), "channel must be stable or main");
    crate::shared::handoff::set_config_value_in(path, "updateChannel", channel.into())
}

fn write_runtime_in(path: &Path, pid: u32, ticks: u64) -> Result<()> {
    let revision = option_env!("GITHUB_SHA").filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()));
    let installed = target()?.and_then(|image| installed_id(&image));
    crate::shared::handoff::set_config_value_in(path, "running", serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"), "revision": revision,
        "pid": pid, "startTicks": ticks,
        "executable": fs::read_link("/proc/self/exe")?,
        "updateId": installed.as_ref().map(|(_, id)| id),
        "updateChannel": installed.as_ref().map(|(channel, _)| channel),
    }))
}

fn process_ticks(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(") ")?.1.split_whitespace().nth(19)?.parse().ok()
}

fn same_executable(current: &Path, recorded: &str) -> bool {
    // Linux appends this suffix to /proc/PID/exe after the AppImage is exchanged.
    // The running process keeps its old version even though its inode moved.
    current.to_str().is_some_and(|path| path.strip_suffix(" (deleted)").unwrap_or(path) == recorded)
}

pub fn record_running() -> Result<()> {
    let pid = std::process::id();
    write_runtime_in(&crate::shared::handoff::default_config_path(), pid,
        process_ticks(pid).context("cannot read Apparent process start time")?)
}

/// Mark the GPUI application as initialized for the update relaunch helper.
pub fn mark_relaunch_ready() -> Result<()> {
    let Some(path) = std::env::var_os("APPARENT_UPDATE_READY").map(PathBuf::from) else { return Ok(()) };
    let image = target()?.context("relaunch readiness requires an AppImage")?;
    let expected = format!(".{}.ready-", image.file_name().unwrap().to_string_lossy());
    ensure!(path.parent() == image.parent() && path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(&expected)),
        "invalid relaunch readiness path");
    OpenOptions::new().write(true).create_new(true).open(path)?.sync_all()?;
    Ok(())
}

pub fn status_at(path: &Path) -> Result<serde_json::Value> {
    let config: serde_json::Value = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(error.into()),
    };
    ensure!(config.is_object(), "config must be an object");
    let channel = if config["updateChannel"] == "main" { "main" } else { "stable" };
    let recorded = &config["running"];
    let running = match (recorded["pid"].as_u64().and_then(|n| u32::try_from(n).ok()), recorded["startTicks"].as_u64()) {
        (Some(pid), Some(ticks)) if process_ticks(pid) == Some(ticks)
            && fs::read_link(format!("/proc/{pid}/exe")).ok().as_deref()
                .zip(recorded["executable"].as_str())
                .is_some_and(|(current, original)| same_executable(current, original)) => recorded.clone(),
        _ => serde_json::Value::Null,
    };
    let image = target()?;
    let installed = image.as_ref().and_then(|path| installed_id(path));
    let pending_restart = installed.as_ref().is_some_and(|(feed, id)| {
        running.is_object() && (running["updateId"].as_str() != Some(id.as_str()) ||
            running["updateChannel"].as_str() != Some(feed.as_str()))
    });
    Ok(serde_json::json!({
        "channel": channel, "running": running,
        "installed": installed.as_ref().map(|(feed, id)| serde_json::json!({"channel": feed, "id": id})),
        "pendingRestart": pending_restart, "canUpdate": image.is_some(),
    }))
}

pub fn status() -> Result<serde_json::Value> {
    status_at(&crate::shared::handoff::default_config_path())
}

pub fn check_cli() -> Result<&'static str> {
    target()?.context("Updates require an AppImage owned by you under your home directory")?;
    let _guard = CHECK.lock().map_err(|_| anyhow::anyhow!("update check lock poisoned"))?;
    Ok(if check()? {
        "installed; restart Apparent to use it"
    } else if status()?["pendingRestart"] == true {
        "already installed; restart Apparent to use it"
    } else {
        "up to date"
    })
}

/// Start the installed AppImage after this process exits, not the old running inode.
pub fn relaunch_staged() -> Result<()> {
    let image = target()?.context("Updates require an owned AppImage under your home directory")?;
    let name = image.file_name().context("AppImage has no name")?.to_string_lossy();
    let backup = image.with_file_name(format!(".{name}.previous"));
    // Own the transaction from validation through post-exit readiness/recovery.
    let mut command = relaunch_command(&image, &backup, std::process::id())?;
    ensure!(status()?["pendingRestart"] == true, "no installed update is waiting for a restart");
    ensure!(backup.is_file(), "previous AppImage is unavailable for recovery");
    ensure!(fs::metadata(&backup)?.permissions().mode() & 0o111 != 0, "previous AppImage is not executable");
    // A successful shell spawn proves nothing about the image it will execute later.
    // Check the new image before leaving the running app, and keep the old one as a
    // recovery path if the image disappears or fails to start after this process exits.
    preflight_image(&image, Duration::from_secs(5))?;
    command.spawn()
        .context("cannot start updated AppImage after exit")?;
    Ok(())
}

fn preflight_image(image: &Path, timeout: Duration) -> Result<()> {
    let mut child = Command::new(image).arg("--help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null()).spawn()
        .context("updated AppImage could not start")?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "updated AppImage failed its startup check");
            return Ok(());
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("updated AppImage startup check timed out");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn file_identity(path: &Path) -> Result<String> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(meta.is_file() && !meta.file_type().is_symlink() && meta.uid() == unsafe { libc::geteuid() },
                "update recovery input must be an owned regular file");
            Ok(format!("{}:{}", meta.dev(), meta.ino()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok("-".into()),
        Err(error) => Err(error.into()),
    }
}

fn marker_contents(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text.trim_end_matches('\n').to_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

fn relaunch_command(image: &Path, backup: &Path, pid: u32) -> Result<Command> {
    let lock = transaction_lock(image)?;
    let marker = image.with_extension("AppImage.apparent-update.json");
    let previous_marker = backup.with_file_name(format!("{}.apparent-update.json", backup.file_name().unwrap().to_string_lossy()));
    let failed_marker = image.with_file_name(format!(".{}.failed-update.json", image.file_name().unwrap().to_string_lossy()));
    let ready = image.with_file_name(format!(".{}.ready-{pid}", image.file_name().unwrap().to_string_lossy()));
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(r#"pid=$1; image=$2; backup=$3; marker=$4; previous_marker=$5; failed_marker=$6; ready=$7
image_id=$8; backup_id=$9; shift 9; marker_id=$1; previous_id=$2; marker_text=$3; previous_text=$4
same() {
    [ ! -L "$1" ] || return 1
    if [ "$2" = '-' ]; then [ ! -e "$1" ]; else [ "$(stat -c '%d:%i' "$1" 2>/dev/null)" = "$2" ]; fi
}
unchanged() {
    { same "$image" "$image_id" || { [ ! -e "$image" ] && [ ! -L "$image" ]; }; } &&
    same "$backup" "$backup_id" && same "$marker" "$marker_id" && same "$previous_marker" "$previous_id" &&
    [ "$(cat "$marker" 2>/dev/null)" = "$marker_text" ] && [ "$(cat "$previous_marker" 2>/dev/null)" = "$previous_text" ]
}
i=0
while kill -0 "$pid" 2>/dev/null && [ "$i" -lt 100 ]; do sleep 0.1; i=$((i+1)); done
kill -0 "$pid" 2>/dev/null && exit 1
unchanged || exit 1
rm -f "$ready"
APPARENT_UPDATE_READY="$ready" "$image" app </dev/null & child=$!
i=0
while [ "$i" -lt 150 ]; do
    if [ -f "$ready" ] && kill -0 "$child" 2>/dev/null; then rm -f "$ready"; exit 0; fi
    if ! kill -0 "$child" 2>/dev/null; then break; fi
    sleep 0.1; i=$((i+1))
done
if kill -0 "$child" 2>/dev/null; then
    kill "$child" 2>/dev/null
    i=0
    while kill -0 "$child" 2>/dev/null && [ "$i" -lt 20 ]; do sleep 0.1; i=$((i+1)); done
    # SIGTERM may only queue cooperative Stop. Do not hold the transaction
    # forever, force-kill a possibly busy app, or replace an image still live.
    kill -0 "$child" 2>/dev/null && exit 1
fi
wait "$child" 2>/dev/null
rm -f "$ready"
# A non-cooperating writer must not make us roll back or blacklist its image.
unchanged || exit 1
if [ -x "$backup" ]; then
    if mv -f "$backup" "$image"; then
        if [ -f "$marker" ]; then mv -f "$marker" "$failed_marker" || rm -f "$marker"; fi
        if [ -f "$previous_marker" ]; then mv -f "$previous_marker" "$marker" || rm -f "$marker"; else rm -f "$marker"; fi
        exec "$image" app </dev/null
    fi
    exec "$backup" app </dev/null
fi
exit 1"#)
        .arg("apparent-update-relaunch")
        .arg(pid.to_string())
        .arg(image)
        .arg(backup)
        .arg(&marker)
        .arg(&previous_marker)
        .arg(failed_marker)
        .arg(ready)
        .arg(file_identity(image)?)
        .arg(file_identity(backup)?)
        .arg(file_identity(&marker)?)
        .arg(file_identity(&previous_marker)?)
        .arg(marker_contents(&marker)?)
        .arg(marker_contents(&previous_marker)?)
        .env_remove("APPIMAGE")
        .env_remove("APPDIR")
        // Carry the same open-file-description lock across exec with no
        // unlock/relock gap. The shell reads its program from -c, not stdin;
        // launched app children reopen /dev/null and cannot retain the lock.
        .stdin(std::process::Stdio::from(lock))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    Ok(command)
}

// Main previews are published from main as GitHub releases.

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    published_at: Option<String>,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

struct Candidate {
    id: String,
    channel: &'static str,
    url: String,
    sha256: String,
}

fn arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        _ => None,
    }
}

fn main_candidate(release: &Release, arch: &str, checksum: &str) -> Result<Candidate> {
    let sha = release.tag_name.strip_prefix("linux-main-").context("not a main preview")?;
    ensure!(sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()), "invalid main revision");
    let suffix = format!("-linux-{arch}.AppImage");
    let image = release.assets.iter().find(|asset| asset.name.starts_with("Apparent-") && asset.name.ends_with(&suffix))
        .context("main preview has no AppImage for this CPU")?;
    let version = image.name.strip_prefix("Apparent-").and_then(|name| name.strip_suffix(&suffix)).unwrap();
    ensure!(!version.is_empty() && version.bytes().all(|b| b.is_ascii_digit() || b == b'.'), "invalid main AppImage version");
    let checksum_asset = release.assets.iter().find(|asset| asset.name == format!("{}.sha256", image.name))
        .context("main preview has no checksum")?;
    let prefix = format!("https://github.com/{REPO}/releases/download/{}/", release.tag_name);
    ensure!(image.browser_download_url == format!("{prefix}{}", image.name)
        && checksum_asset.browser_download_url == format!("{prefix}{}", checksum_asset.name), "unexpected main preview asset URL");
    let (hash, name) = checksum.trim().split_once(char::is_whitespace).context("invalid main preview checksum")?;
    ensure!(name.trim() == image.name, "checksum refers to another file");
    validate_hash(hash)?;
    Ok(Candidate { id: sha.to_owned(), channel: "main", url: image.browser_download_url.clone(), sha256: hash.to_owned() })
}

fn main_releases(releases: Vec<Release>, arch: &str) -> Vec<Release> {
    let suffix = format!("-linux-{arch}.AppImage");
    let mut releases: Vec<_> = releases.into_iter().filter(|release| {
        release.prerelease && !release.draft && release.published_at.is_some()
            && release.tag_name.strip_prefix("linux-main-").is_some_and(|sha| {
                sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit())
            })
            && release.assets.iter().any(|asset| {
                asset.name.starts_with("Apparent-") && asset.name.ends_with(&suffix)
                    && release.assets.iter().any(|sum| sum.name == format!("{}.sha256", asset.name))
            })
    }).collect();
    releases.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    releases
}

fn candidate_from_release(release: &Release, arch: &str, fetch: &impl Fn(&str) -> Result<Vec<u8>>) -> Result<Candidate> {
    let suffix = format!("-linux-{arch}.AppImage.sha256");
    let checksum = release.assets.iter().find(|asset| asset.name.starts_with("Apparent-") && asset.name.ends_with(&suffix))
        .context("main preview has no checksum for this CPU")?;
    let prefix = format!("https://github.com/{REPO}/releases/download/{}/", release.tag_name);
    ensure!(checksum.browser_download_url == format!("{prefix}{}", checksum.name), "unexpected main preview checksum URL");
    let text = fetch(&checksum.browser_download_url)?;
    main_candidate(&release, arch, std::str::from_utf8(&text)?)
}

fn first_usable_main(releases: Vec<Release>, arch: &str, fetch: &impl Fn(&str) -> Result<Vec<u8>>) -> Result<Option<Candidate>> {
    let mut last_error = None;
    for release in main_releases(releases, arch) {
        match candidate_from_release(&release, arch, fetch) {
            Ok(candidate) => return Ok(Some(candidate)),
            Err(error) => last_error = Some(error),
        }
    }
    match last_error {
        Some(error) => Err(error),
        None => Ok(None),
    }
}

fn latest_main_candidate(arch: &str) -> Result<Candidate> {
    // Release lists include Mac previews, old Scrawl assets and stable releases.
    // GitHub pages by creation date, not publication date: inspect all pages before choosing.
    let mut releases = Vec::new();
    for page in 1.. {
        let body = get(&format!("https://api.github.com/repos/{REPO}/releases?per_page=10&page={page}"), None)?;
        let page_releases: Vec<Release> = serde_json::from_slice(&body).context("invalid release list")?;
        if page_releases.is_empty() { break; }
        releases.extend(page_releases);
    }
    first_usable_main(releases, arch, &|url| get(url, None))?
        .context("no published Linux main preview yet")
}

fn validate_hash(hash: &str) -> Result<()> {
    ensure!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()), "invalid SHA-256");
    Ok(())
}

fn get(url: &str, out: Option<&Path>) -> Result<Vec<u8>> {
    ensure!(url.starts_with("https://"), "update URL must use HTTPS");
    let mut cmd = Command::new("curl");
    cmd.args(["--fail", "--silent", "--show-error", "--location", "--proto", "=https", "--proto-redir", "=https", "--connect-timeout", "10", "--max-time", "300", "--user-agent", "Apparent", "--url", url]);
    if let Some(path) = out {
        cmd.args(["--max-filesize", "1073741824"]);
        cmd.arg("--output").arg(path);
    } else {
        // A page of 100 releases includes component assets and can exceed 256 KiB.
        // Keep metadata bounded while avoiding dozens of anonymous API requests.
        cmd.args(["--max-filesize", "4194304"]);
    }
    let result = cmd.output().context("curl is required for updates")?;
    ensure!(result.status.success(), "download failed: {}", String::from_utf8_lossy(&result.stderr));
    Ok(result.stdout)
}

fn stable_candidate(arch: &str) -> Result<Option<Candidate>> {
    stable_candidate_with(arch, &|url| get(url, None))
}

fn stable_candidate_with(arch: &str, fetch: &impl Fn(&str) -> Result<Vec<u8>>) -> Result<Option<Candidate>> {
    let mut releases = Vec::new();
    let mut complete = false;
    for page in 1..=10 {
        let body = fetch(&format!("https://api.github.com/repos/{REPO}/releases?per_page=100&page={page}"))?;
        let page_releases: Vec<Release> = serde_json::from_slice(&body).context("invalid stable release list")?;
        ensure!(page_releases.len() <= 100, "oversized stable release page");
        complete = page_releases.len() < 100;
        releases.extend(page_releases);
        if complete { break; }
    }
    ensure!(complete, "stable release discovery exceeded its page limit");

    // GitHub's /latest can name a standalone component instead of the desktop app.
    // Only published versioned desktop releases belong to the Stable channel.
    let mut releases: Vec<_> = releases.into_iter().filter_map(|release| {
        if release.draft || release.prerelease || release.published_at.is_none() { return None; }
        let version = release.tag_name.strip_prefix('v')?.split('.')
            .map(str::parse::<u32>).collect::<Result<Vec<_>, _>>().ok()?;
        let name = format!("Apparent-{}-linux-{arch}.AppImage", &release.tag_name[1..]);
        (release.assets.iter().any(|asset| asset.name == name)
            && release.assets.iter().any(|asset| asset.name == format!("{name}.sha256")))
            .then_some((version, release))
    }).collect();
    releases.sort_by(|a, b| b.0.cmp(&a.0));
    let Some((_, release)) = releases.into_iter().next() else { return Ok(None); };
    let name = format!("Apparent-{}-linux-{arch}.AppImage", &release.tag_name[1..]);
    let image = release.assets.iter().find(|a| a.name == name).context("no Linux AppImage in the stable release yet")?;
    let checksum = release.assets.iter().find(|a| a.name == format!("{name}.sha256")).context("stable release has no checksum")?;
    let prefix = format!("https://github.com/{REPO}/releases/download/{}/", release.tag_name);
    ensure!(image.browser_download_url == format!("{prefix}{name}") && checksum.browser_download_url == format!("{prefix}{name}.sha256"), "unexpected release asset URL");
    let body = fetch(&checksum.browser_download_url)?;
    let text = std::str::from_utf8(&body)?;
    let (hash, file) = text.trim().split_once(char::is_whitespace).context("invalid release checksum")?;
    ensure!(file.trim() == name, "checksum refers to another file");
    validate_hash(hash)?;
    Ok(Some(Candidate { id: release.tag_name, channel: "stable", url: image.browser_download_url.clone(), sha256: hash.to_owned() }))
}

fn installed_id(image: &Path) -> Option<(String, String)> {
    update_id(&image.with_extension("AppImage.apparent-update.json"))
}

fn update_id(marker: &Path) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(marker).ok()?).ok()?;
    Some((value["channel"].as_str()?.to_owned(), value["id"].as_str()?.to_owned()))
}

fn previously_failed(marker: &Path, candidate: &Candidate) -> bool {
    update_id(marker) == Some((candidate.channel.to_owned(), candidate.id.clone()))
}

fn newer(candidate: &Candidate, installed: Option<(String, String)>) -> bool {
    let version = |s: &str| {
        s.trim_start_matches('v').split('.').map(str::parse::<u32>).collect::<Result<Vec<_>, _>>()
    };
    if let Some((channel, id)) = installed {
        if channel == candidate.channel {
            if channel == "stable" {
                return matches!((version(&candidate.id), version(&id)), (Ok(r), Ok(c)) if r > c);
            }
            return id != candidate.id;
        }
        // Switching channels installs the requested channel even when version strings match.
        return true;
    }
    if candidate.channel == "stable" {
        return matches!((version(&candidate.id), version(env!("CARGO_PKG_VERSION"))), (Ok(r), Ok(c)) if r > c);
    }
    // Old AppImages have no revision embedded; opt-in Main replaces them once.
    true
}

fn main_advances(current: Option<&str>, candidate: &Candidate, fetch: &impl Fn(&str) -> Result<Vec<u8>>) -> Result<bool> {
    if candidate.channel != "main" { return Ok(true) }
    let Some(current) = current else { return Ok(true) };
    ensure!([current, &candidate.id].iter().all(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit())),
        "invalid Main update revision");
    if current == candidate.id { return Ok(false) }
    // Page two retains comparison metadata but omits GitHub's potentially huge
    // first-page file patches. Commit dates and publication times are not ancestry.
    let body = fetch(&format!("https://api.github.com/repos/{REPO}/compare/{current}...{}?per_page=1&page=2", candidate.id))?;
    let comparison: serde_json::Value = serde_json::from_slice(&body).context("invalid Main comparison")?;
    ensure!(comparison["base_commit"]["sha"].as_str() == Some(current), "unexpected Main comparison base");
    match comparison["status"].as_str() {
        Some("ahead") => Ok(true),
        Some("behind" | "identical") => Ok(false),
        _ => anyhow::bail!("Main update is not a verified descendant of the installed revision"),
    }
}

fn target() -> Result<Option<PathBuf>> {
    let Some(path) = std::env::var_os("APPIMAGE").map(PathBuf::from) else { return Ok(None) };
    let meta = fs::symlink_metadata(&path)?;
    ensure!(meta.is_file() && !meta.file_type().is_symlink(), "update target must be an ordinary AppImage");
    ensure!(meta.uid() == unsafe { libc::geteuid() }, "update target belongs to another user");
    let home = dirs::home_dir().context("home directory unavailable")?;
    let path = path.canonicalize()?;
    ensure!(path.starts_with(home.canonicalize()?), "refusing to replace an AppImage outside your home");
    Ok(Some(path))
}

fn verify_download(stage: &Path, hash: &str) -> Result<()> {
    let mut hasher = Sha256::new();
    let mut file = File::open(stage)?;
    let mut bytes = [0u8; 128 * 1024];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 { break; }
        hasher.update(&bytes[..n]);
    }
    ensure!(format!("{:x}", hasher.finalize()) == hash, "AppImage checksum mismatch");
    let mut perms = fs::metadata(stage)?.permissions();
    perms.set_mode(0o700);
    fs::set_permissions(stage, perms)?;
    File::open(stage)?.sync_all()?;
    Ok(())
}

fn exchange(stage: &Path, image: &Path) -> Result<()> {
    let a = std::ffi::CString::new(stage.as_os_str().as_encoded_bytes())?;
    let b = std::ffi::CString::new(image.as_os_str().as_encoded_bytes())?;
    let rc = unsafe { libc::renameat2(libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), libc::RENAME_EXCHANGE) };
    ensure!(rc == 0, "atomic AppImage swap failed: {}", std::io::Error::last_os_error());
    Ok(())
}

fn install(candidate: &Candidate, image: &Path) -> Result<()> {
    let parent = image.parent().context("AppImage has no parent")?;
    let name = image.file_name().context("AppImage has no name")?.to_string_lossy();
    let stage = parent.join(format!(".{name}.download-{}", std::process::id()));
    let backup = parent.join(format!(".{name}.previous"));
    let sidecar = image.with_extension("AppImage.apparent-update.json");
    let previous_sidecar = parent.join(format!(".{name}.previous.apparent-update.json"));
    let staged_sidecar = parent.join(format!(".{name}.update-{}", std::process::id()));
    // Reserve only our own temporary path; never reuse another process's download.
    let _reserved = OpenOptions::new().write(true).create_new(true).open(&stage)?;
    let mut swapped = false;
    let result = (|| -> Result<()> {
        get(&candidate.url, Some(&stage))?;
        verify_download(&stage, &candidate.sha256)?;
        // Fail closed if the launcher changed while the download was in flight.
        let current = fs::symlink_metadata(image)?;
        ensure!(current.is_file() && !current.file_type().is_symlink() && current.uid() == unsafe { libc::geteuid() }, "installed AppImage changed while downloading");
        let config = crate::shared::handoff::load_config_from(&crate::shared::handoff::default_config_path());
        let selected = if config.update_channel.as_deref() == Some("main") { "main" } else { "stable" };
        ensure!(selected == candidate.channel, "channel changed while downloading");
        let mut f = OpenOptions::new().write(true).create_new(true).open(&staged_sidecar)?;
        f.write_all(serde_json::to_string(&serde_json::json!({"channel": candidate.channel, "id": candidate.id}))?.as_bytes())?;
        f.sync_all()?;
        // Keep the old version marker with its old image, or remove a stale one.
        if fs::symlink_metadata(&previous_sidecar).is_ok() {
            fs::remove_file(&previous_sidecar)?;
        }
        match fs::symlink_metadata(&sidecar) {
            Ok(meta) => {
                ensure!(meta.is_file() && !meta.file_type().is_symlink() && meta.uid() == unsafe { libc::geteuid() }, "update marker is not an owned regular file");
                fs::copy(&sidecar, &previous_sidecar)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
        exchange(&stage, image)?;
        swapped = true;
        // The old AppImage is now at stage; it remains a rollback copy.
        if let Err(error) = fs::rename(&stage, &backup) {
            exchange(&stage, image).context("could not restore installed AppImage after backup failed")?;
            swapped = false;
            return Err(error).context("could not retain the previous AppImage");
        }
        if let Err(error) = fs::rename(&staged_sidecar, &sidecar) {
            exchange(&backup, image).context("could not restore installed AppImage after marker failed")?;
            let _ = fs::remove_file(&backup);
            return Err(error).context("could not publish the updated version marker");
        }
        Ok(())
    })();
    // If the swap succeeded, stage no longer exists; never touch the installed image here.
    if !swapped && stage.exists() { let _ = fs::remove_file(&stage); }
    if staged_sidecar.exists() { let _ = fs::remove_file(&staged_sidecar); }
    result
}

fn transaction_lock(image: &Path) -> Result<File> {
    let name = image.file_name().context("AppImage has no name")?.to_string_lossy();
    let path = image.with_file_name(format!(".{name}.lock"));
    // Do not unlink this inode on release: a waiter must never lock a different
    // file while the previous transaction still holds the original one.
    let lock = OpenOptions::new().read(true).write(true).create(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path)?;
    let meta = lock.metadata()?;
    ensure!(meta.is_file() && meta.uid() == unsafe { libc::geteuid() }
        && meta.nlink() == 1 && meta.mode() & 0o077 == 0, "update lock must be a private owned file");
    ensure!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "another update transaction is running: {}", std::io::Error::last_os_error());
    Ok(lock)
}

fn check() -> Result<bool> {
    let Some(image) = target()? else { return Ok(false) };
    // CLI processes and the GUI's hourly/manual worker share this lock. Acquire
    // before reading identity or discovering a release; hold through the swap.
    let _transaction = transaction_lock(&image)?;
    let status = status()?;
    let arch = arch().context("no AppImage for this CPU")?;
    let candidate = if status["channel"] == "main" {
        latest_main_candidate(arch)?
    } else {
        let Some(candidate) = stable_candidate(arch)? else { return Ok(false); };
        candidate
    };
    let name = image.file_name().context("AppImage has no name")?.to_string_lossy();
    let failed = image.with_file_name(format!(".{name}.failed-update.json"));
    ensure!(!previously_failed(&failed, &candidate), "this update failed to start previously; waiting for a newer release");
    let installed = installed_id(&image);
    if !newer(&candidate, installed.clone()) { return Ok(false) }
    let current = match &installed {
        Some((channel, id)) if channel == "main" => Some(id.as_str()),
        // No marker yet: a freshly packaged Main build already knows its SHA.
        None => option_env!("GITHUB_SHA").filter(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit())),
        _ => None, // An explicit channel switch keeps its existing semantics.
    };
    if !main_advances(current, &candidate, &|url| get(url, None))? { return Ok(false) }
    install(&candidate, &image)?;
    super::super::integration::shell::notify("Apparent updated", "The new version will start the next time you open Apparent. Your previous AppImage is kept beside it.");
    Ok(true)
}

pub fn check_now() {
    if let Ok(_guard) = CHECK.try_lock()
        && let Err(error) = check()
    {
        crate::capture::engine::runtime::apparent_log(format!("Linux update: {error:#}"));
    }
}

pub fn start() {
    if std::env::var_os("APPIMAGE").is_none() { return }
    std::thread::spawn(|| loop {
        check_now();
        std::thread::sleep(Duration::from_secs(3600));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_transaction_lock_child() {
        let Some(image) = std::env::var_os("APPARENT_TEST_UPDATE_LOCK_IMAGE").map(PathBuf::from) else { return };
        let mode = std::env::var("APPARENT_TEST_UPDATE_LOCK_MODE").unwrap();
        let lock = transaction_lock(&image);
        if mode == "contended" {
            assert!(lock.is_err(), "other process must not enter discovery/download/swap");
            return;
        }
        let _lock = lock.unwrap();
        let candidate = Candidate { id: "b".repeat(40), channel: "main", url: String::new(), sha256: String::new() };
        let (channel, current) = installed_id(&image).unwrap();
        assert_eq!(channel, "main");
        assert_eq!(current, "c".repeat(40));
        assert!(!main_advances(Some(&current), &candidate, &|_| Ok(serde_json::to_vec(
            &serde_json::json!({"base_commit":{"sha":current},"status":"behind"}))?)).unwrap());
        assert_eq!(fs::read(&image).unwrap(), b"newer C image", "stale B must not swap over C");
    }

    #[test]
    fn updater_processes_serialize_before_identity_selection_and_cannot_finish_stale() {
        let dir = std::env::temp_dir().join(format!("apparent-update-lock-{}", crate::shared::ids::make_bundle_id()));
        fs::create_dir(&dir).unwrap();
        let image = dir.join("Apparent.AppImage");
        fs::write(&image, b"old A image").unwrap();
        let run_child = |mode: &str| {
            let test = format!("{}::update_transaction_lock_child", module_path!().split_once("::").unwrap().1);
            let output = Command::new(std::env::current_exe().unwrap()).args(["--exact", &test, "--nocapture"])
                .env("APPARENT_TEST_UPDATE_LOCK_IMAGE", &image).env("APPARENT_TEST_UPDATE_LOCK_MODE", mode)
                .output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"), "child test must actually execute");
        };
        let lock = transaction_lock(&image).unwrap();
        let inode = fs::metadata(image.with_file_name(".Apparent.AppImage.lock")).unwrap().ino();
        run_child("contended");
        let stage = dir.join("C.download");
        fs::write(&stage, b"newer C image").unwrap();
        exchange(&stage, &image).unwrap();
        fs::write(image.with_extension("AppImage.apparent-update.json"), serde_json::to_vec(
            &serde_json::json!({"channel":"main","id":"c".repeat(40)})).unwrap()).unwrap();
        drop(lock);
        run_child("stale");
        assert_eq!(fs::metadata(image.with_file_name(".Apparent.AppImage.lock")).unwrap().ino(), inode);
        assert_eq!(fs::read(&stage).unwrap(), b"old A image");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn transaction_lock_rejects_symlinks_hardlinks_and_public_files() {
        let dir = std::env::temp_dir().join(format!("apparent-lock-safety-{}", crate::shared::ids::make_bundle_id()));
        fs::create_dir(&dir).unwrap();
        let image = dir.join("image");
        let path = dir.join(".image.lock");
        let other = dir.join("other");
        fs::write(&other, b"untouched").unwrap();
        std::os::unix::fs::symlink(&other, &path).unwrap();
        assert!(transaction_lock(&image).is_err());
        fs::remove_file(&path).unwrap();
        fs::hard_link(&other, &path).unwrap();
        assert!(transaction_lock(&image).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"untouched").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(transaction_lock(&image).is_err());
        assert_eq!(fs::read(&other).unwrap(), b"untouched");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn main_updates_require_forward_ancestry_not_different_shas() {
        let current = "a".repeat(40);
        let candidate = Candidate { id: "b".repeat(40), channel: "main", url: String::new(), sha256: String::new() };
        for (status, expected) in [("ahead", Some(true)), ("behind", Some(false)), ("identical", Some(false)), ("diverged", None), ("unknown", None)] {
            let result = main_advances(Some(&current), &candidate, &|url| {
                assert_eq!(url, format!("https://api.github.com/repos/{REPO}/compare/{current}...{}?per_page=1&page=2", candidate.id));
                Ok(serde_json::to_vec(&serde_json::json!({"status":status,"base_commit":{"sha":current}}))?)
            });
            match expected {
                Some(expected) => assert_eq!(result.unwrap(), expected),
                None => assert!(result.is_err()),
            }
        }
        assert!(main_advances(Some(&current), &candidate, &|_| anyhow::bail!("offline")).is_err());
        assert!(main_advances(Some(&current), &candidate, &|_| Ok(b"{}".to_vec())).is_err());
        assert!(main_advances(Some(&current), &candidate, &|_| Ok(serde_json::to_vec(&serde_json::json!({"status":"ahead","base_commit":{"sha":candidate.id}}))?)).is_err());
        assert!(main_advances(Some("bad-revision"), &candidate, &|_| panic!("must validate before fetch")).is_err());
        assert!(!main_advances(Some(&candidate.id), &candidate, &|_| panic!("same revision needs no request")).unwrap());
        assert!(main_advances(None, &candidate, &|_| panic!("legacy migration has no baseline")).unwrap());
        let stable = Candidate { channel: "stable", ..candidate };
        assert!(main_advances(Some(&current), &stable, &|_| panic!("Stable must not use Main comparison")).unwrap());
    }

    #[test]
    fn main_channel_rejects_other_architecture_and_unpublished_releases() {
        let tag = format!("linux-main-{}", "a".repeat(40));
        let image = "Apparent-0.3.13-linux-aarch64.AppImage";
        let release = || Release {
            tag_name: tag.clone(), prerelease: true, draft: false, published_at: Some("2026-11-01T01:00:00Z".into()),
            assets: [image.to_owned(), format!("{image}.sha256")].into_iter().map(|name| ReleaseAsset {
                browser_download_url: format!("https://github.com/{REPO}/releases/download/{tag}/{name}"), name,
            }).collect(),
        };
        assert!(first_usable_main(vec![release()], "x86_64", &|_| panic!("foreign architecture must not fetch")).unwrap().is_none());
        let mut draft = release();
        draft.draft = true;
        let mut unpublished = release();
        unpublished.published_at = None;
        assert!(first_usable_main(vec![draft, unpublished], "aarch64", &|_| panic!("private releases must not fetch")).unwrap().is_none());
        let candidate = first_usable_main(vec![release()], "aarch64", &|_| Ok(format!("{}  {image}", "c".repeat(64)).into_bytes())).unwrap().unwrap();
        assert!(candidate.url.ends_with("-linux-aarch64.AppImage"));
    }

    fn stable_release_fixture(tag: &str, arch: &str) -> serde_json::Value {
        let name = format!("Apparent-{}-linux-{arch}.AppImage", tag.strip_prefix('v').unwrap_or(tag));
        let prefix = format!("https://github.com/{REPO}/releases/download/{tag}/");
        serde_json::json!({"tag_name": tag, "draft": false, "prerelease": false,
            "published_at": "2026-11-01T00:00:00Z", "assets": [
                {"name": name, "browser_download_url": format!("{prefix}{name}")},
                {"name": format!("{name}.sha256"), "browser_download_url": format!("{prefix}{name}.sha256")}]})
    }

    #[test]
    fn stable_discovers_desktop_versions_after_component_release_pages() {
        let candidate = stable_candidate_with("x86_64", &|url| {
            if url.ends_with("?per_page=100&page=1") {
                return Ok(serde_json::to_vec(&vec![stable_release_fixture("recipient-component", "x86_64"); 100])?);
            }
            if url.ends_with("?per_page=100&page=2") {
                return Ok(serde_json::to_vec(&vec![stable_release_fixture("v0.3.9", "x86_64"),
                    stable_release_fixture("v0.3.14", "x86_64"), stable_release_fixture("v0.3.10", "x86_64")])?);
            }
            assert_eq!(url, format!("https://github.com/{REPO}/releases/download/v0.3.14/Apparent-0.3.14-linux-x86_64.AppImage.sha256"));
            Ok(format!("{}  Apparent-0.3.14-linux-x86_64.AppImage", "c".repeat(64)).into_bytes())
        }).unwrap().unwrap();
        assert_eq!(candidate.id, "v0.3.14");
        assert_eq!(candidate.channel, "stable");
        assert_eq!(candidate.sha256, "c".repeat(64));
    }

    #[test]
    fn stable_with_no_compatible_desktop_release_has_no_update() {
        let mut preview = stable_release_fixture("v0.3.15", "x86_64");
        preview["prerelease"] = true.into();
        let mut draft = stable_release_fixture("v0.3.15", "x86_64");
        draft["draft"] = true.into();
        let mut unpublished = stable_release_fixture("v0.3.15", "x86_64");
        unpublished["published_at"] = serde_json::Value::Null;
        let mut incomplete = stable_release_fixture("v0.3.15", "x86_64");
        incomplete["assets"].as_array_mut().unwrap().pop();
        let mut legacy = stable_release_fixture("v0.3.13", "x86_64");
        for asset in legacy["assets"].as_array_mut().unwrap() {
            asset["name"] = asset["name"].as_str().unwrap().replace("Apparent-", "Scrawl-").into();
        }
        let releases = vec![preview, draft, unpublished, incomplete, legacy,
            stable_release_fixture("v0.3.15", "aarch64"),
            stable_release_fixture("preview-mac-12-2", "x86_64"),
            stable_release_fixture("recipient-component", "x86_64")];
        assert!(stable_candidate_with("x86_64", &|url| {
            assert!(url.ends_with("?per_page=100&page=1"), "no checksum or download may be requested");
            Ok(serde_json::to_vec(&releases)?)
        }).unwrap().is_none());
    }

    #[test]
    fn stable_network_metadata_and_integrity_failures_remain_errors() {
        assert!(stable_candidate_with("x86_64", &|_| anyhow::bail!("offline")).is_err());
        assert!(stable_candidate_with("x86_64", &|_| Ok(b"not JSON".to_vec())).is_err());
        for checksum in ["invalid".to_owned(), format!("{}  unrelated.AppImage", "c".repeat(64))] {
            assert!(stable_candidate_with("x86_64", &|url| {
                if url.contains("/releases?") {
                    Ok(serde_json::to_vec(&vec![stable_release_fixture("v0.3.14", "x86_64")])?)
                } else { Ok(checksum.clone().into_bytes()) }
            }).is_err());
        }
        let mut foreign = stable_release_fixture("v0.3.14", "x86_64");
        foreign["assets"][0]["browser_download_url"] = "https://untrusted.example/app.AppImage".into();
        assert!(stable_candidate_with("x86_64", &|url| {
            assert!(url.contains("/releases?"), "foreign asset must be rejected before fetching");
            Ok(serde_json::to_vec(&vec![foreign.clone()])?)
        }).is_err());
        assert!(stable_candidate_with("x86_64", &|_| {
            Ok(serde_json::to_vec(&vec![stable_release_fixture("recipient-component", "x86_64"); 100])?)
        }).is_err(), "incomplete discovery cannot be reported as no update");
    }

    #[test]
    fn main_channel_selects_latest_published_main_release() {
        let release = |tag: &str, date: &str, draft: bool, name: &str| Release {
            tag_name: tag.into(), prerelease: true, draft, published_at: Some(date.into()),
            assets: [name.to_owned(), format!("{name}.sha256")].into_iter().map(|name| ReleaseAsset {
                browser_download_url: format!("https://example.org/{name}"), name,
            }).collect(),
        };
        let apparent = "Apparent-0.3.13-linux-x86_64.AppImage";
        let releases = vec![
            release("preview-mac-1", "2026-10-29T01:00:00Z", false, apparent),
            release(&format!("linux-main-{}", "a".repeat(40)), "2026-10-27T01:00:00Z", false, apparent),
            release(&format!("linux-main-{}", "b".repeat(40)), "2026-10-28T01:00:00Z", false, apparent),
            release(&format!("linux-main-{}", "c".repeat(40)), "2026-10-29T01:00:00Z", true, apparent),
            release(&format!("linux-main-{}", "d".repeat(40)), "2026-10-30T01:00:00Z", false,
                "Scrawl-0.3.13-linux-x86_64.AppImage"),
        ];
        assert_eq!(main_releases(releases, "x86_64")[0].tag_name,
            "linux-main-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        assert!(main_releases(Vec::new(), "x86_64").is_empty());
    }

    #[test]
    fn main_channel_falls_back_when_newest_preview_is_incomplete() {
        let image = format!("Apparent-{}-linux-x86_64.AppImage", env!("CARGO_PKG_VERSION"));
        let release = |letter: &str, date: &str| {
            let tag = format!("linux-main-{}", letter.repeat(40));
            let url = format!("https://github.com/{REPO}/releases/download/{tag}/");
            Release {
                tag_name: tag, prerelease: true, draft: false, published_at: Some(date.into()),
                assets: [image.clone(), format!("{image}.sha256")].into_iter().map(|name| ReleaseAsset {
                    browser_download_url: format!("{url}{name}"), name,
                }).collect(),
            }
        };
        let releases = vec![
            release("a", "2026-10-28T01:00:00Z"),
            release("b", "2026-10-29T01:00:00Z"),
        ];
        let candidate = first_usable_main(releases, "x86_64", &|url| {
            if url.contains(&format!("linux-main-{}", "b".repeat(40))) {
                Ok(b"invalid checksum".to_vec())
            } else {
                Ok(format!("{}  {image}", "c".repeat(64)).into_bytes())
            }
        }).unwrap().unwrap();
        assert_eq!(candidate.id, "a".repeat(40));
    }

    #[test]
    fn main_channel_orders_publications_across_release_pages() {
        let image = "Apparent-0.3.13-linux-x86_64.AppImage";
        let release = |letter: &str, date: &str| {
            let tag = format!("linux-main-{}", letter.repeat(40));
            Release {
                tag_name: tag.clone(), prerelease: true, draft: false, published_at: Some(date.into()),
                assets: [image.to_owned(), format!("{image}.sha256")].into_iter().map(|name| ReleaseAsset {
                    browser_download_url: format!("https://github.com/{REPO}/releases/download/{tag}/{name}"), name,
                }).collect(),
            }
        };
        // A later page can contain a later publication when GitHub orders by tag creation.
        let pages = [
            vec![release("a", "2026-10-28T01:00:00Z")],
            vec![release("b", "2026-10-29T01:00:00Z")],
        ];
        let candidate = first_usable_main(pages.into_iter().flatten().collect(), "x86_64", &|_| {
            Ok(format!("{}  {image}", "c".repeat(64)).into_bytes())
        }).unwrap().unwrap();
        assert_eq!(candidate.id, "b".repeat(40));
    }

    #[test]
    fn main_release_rejects_foreign_name_and_digest() {
        let expected = format!("Apparent-{}-linux-x86_64.AppImage", env!("CARGO_PKG_VERSION"));
        let url = format!("https://github.com/{REPO}/releases/download/linux-main-{}/", "a".repeat(40));
        let release = |name: &str| Release {
            tag_name: format!("linux-main-{}", "a".repeat(40)),
            prerelease: true, draft: false, published_at: Some("2026-10-28T01:00:00Z".into()),
            assets: [name.to_owned(), format!("{expected}.sha256")].iter().map(|name| ReleaseAsset {
                name: name.clone(), browser_download_url: format!("{url}{name}"),
            }).collect(),
        };
        let checksum = format!("{}  {expected}", "b".repeat(64));
        assert!(main_candidate(&release("other.AppImage"), "x86_64", &checksum).is_err());
        assert!(main_candidate(&release(&expected), "x86_64", "invalid").is_err());
        let candidate = main_candidate(&release(&expected), "x86_64", &checksum).unwrap();
        assert!(candidate.url.contains("linux-main-"));
        assert!(main_candidate(&release(&expected), "x86_64", &format!("{}  other.AppImage", "b".repeat(64))).is_err());
        let mut foreign = release(&expected);
        foreign.assets[1].browser_download_url = "https://example.org/checksum".into();
        assert!(main_candidate(&foreign, "x86_64", &checksum).is_err());
        assert!(!newer(&candidate, Some(("main".into(), "a".repeat(40)))));
        assert!(newer(&candidate, Some(("stable".into(), "v0.3.13".into()))));
    }

    #[test]
    fn hung_updated_image_fails_preflight_without_waiting_for_it() {
        let dir = std::env::temp_dir().join(format!("apparent-preflight-{}", crate::shared::ids::make_bundle_id()));
        fs::create_dir(&dir).unwrap();
        let image = dir.join("hung.AppImage");
        fs::write(&image, b"#!/bin/sh\nexec sleep 5\n").unwrap();
        let mut permissions = fs::metadata(&image).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&image, permissions).unwrap();
        let start = Instant::now();
        assert!(preflight_image(&image, Duration::from_millis(100)).unwrap_err()
            .to_string().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(2));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn relaunch_recovery_serializes_updates_and_never_blacklists_a_different_image() {
        for bypass_lock in [false, true] {
            let dir = std::env::temp_dir().join(format!("apparent-recovery-race-{}", crate::shared::ids::make_bundle_id()));
            fs::create_dir(&dir).unwrap();
            let image = dir.join("Apparent.AppImage");
            let backup = dir.join(".Apparent.AppImage.previous");
            let marker = image.with_extension("AppImage.apparent-update.json");
            let previous_marker = dir.join(".Apparent.AppImage.previous.apparent-update.json");
            let failed_marker = dir.join(".Apparent.AppImage.failed-update.json");
            let started = dir.join("started");
            let fail = dir.join("fail");
            let recovered = dir.join("recovered");
            fs::write(&image, b"#!/bin/sh\ntouch \"$TEST_STARTED\"\nwhile [ ! -f \"$TEST_FAIL\" ]; do sleep 0.02; done\nexit 1\n").unwrap();
            fs::write(&backup, b"#!/bin/sh\ntouch \"$TEST_RECOVERED\"\n").unwrap();
            for path in [&image, &backup] { fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap(); }
            let identity = |id: &str| serde_json::to_vec(&serde_json::json!({"channel":"main","id":id.repeat(40)})).unwrap();
            fs::write(&marker, identity("b")).unwrap();
            fs::write(&previous_marker, identity("a")).unwrap();
            let mut command = relaunch_command(&image, &backup, u32::MAX).unwrap();
            let mut helper = command.env("TEST_STARTED", &started).env("TEST_FAIL", &fail)
                .env("TEST_RECOVERED", &recovered).spawn().unwrap();
            drop(command); // Only the post-exit helper now holds the flock.
            let deadline = Instant::now() + Duration::from_secs(5);
            while !started.exists() && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(10)); }
            assert!(started.exists(), "actual attempted B must reach its gated startup");
            assert!(transaction_lock(&image).is_err(), "C cannot stage while B readiness/recovery is unresolved");
            assert!(relaunch_command(&image, &backup, u32::MAX).is_err(), "competing restart must also refuse");
            if bypass_lock {
                // An external writer deliberately ignores the lock: production
                // exchange/backup operations must not fool rollback identity.
                let stage = dir.join("C.download");
                fs::write(&stage, b"newer C image").unwrap();
                exchange(&stage, &image).unwrap();
                fs::rename(&stage, &backup).unwrap();
                fs::copy(&marker, &previous_marker).unwrap();
                let next_marker = dir.join("C.marker");
                fs::write(&next_marker, identity("c")).unwrap();
                fs::rename(next_marker, &marker).unwrap();
            }
            fs::write(&fail, b"fail B now").unwrap();
            assert_eq!(helper.wait().unwrap().success(), !bypass_lock);
            let _lock = transaction_lock(&image).expect("helper must release its lock on either outcome");
            if bypass_lock {
                assert_eq!(fs::read(&image).unwrap(), b"newer C image");
                assert_eq!(installed_id(&image), Some(("main".into(), "c".repeat(40))));
                assert!(!failed_marker.exists(), "C was never attempted and cannot be blacklisted");
                assert!(!recovered.exists());
            } else {
                assert!(recovered.exists());
                assert_eq!(installed_id(&image), Some(("main".into(), "a".repeat(40))));
                assert_eq!(fs::read(&failed_marker).unwrap(), identity("b"));
            }
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn startup_timeout_releases_lock_without_killing_a_live_term_handler() {
        let dir = std::env::temp_dir().join(format!("apparent-startup-term-{}", crate::shared::ids::make_bundle_id()));
        fs::create_dir(&dir).unwrap();
        let tools = dir.join("tools");
        fs::create_dir(&tools).unwrap();
        // Execute the production helper verbatim. Accelerate only its sleep
        // utility, not its readiness/termination policy or real signal effects.
        let sleep = tools.join("sleep");
        fs::write(&sleep, b"#!/bin/sh\nexec /bin/sleep 0.001\n").unwrap();
        fs::set_permissions(&sleep, fs::Permissions::from_mode(0o700)).unwrap();
        let image = dir.join("Apparent.AppImage");
        let backup = dir.join(".Apparent.AppImage.previous");
        let marker = image.with_extension("AppImage.apparent-update.json");
        let previous_marker = dir.join(".Apparent.AppImage.previous.apparent-update.json");
        let failed_marker = dir.join(".Apparent.AppImage.failed-update.json");
        let started = dir.join("started");
        let term = dir.join("term-requested");
        let recovered = dir.join("recovered");
        fs::write(&image, b"#!/bin/sh\ntrap 'touch \"$TEST_TERM\"' TERM\nprintf '%s' \"$$\" > \"$TEST_STARTED\"\nwhile :; do /bin/sleep 0.02; done\n").unwrap();
        fs::write(&backup, b"#!/bin/sh\ntouch \"$TEST_RECOVERED\"\n").unwrap();
        for path in [&image, &backup] { fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap(); }
        let next = br#"{"channel":"main","id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}"#;
        let previous = br#"{"channel":"main","id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;
        fs::write(&marker, next).unwrap();
        fs::write(&previous_marker, previous).unwrap();
        let backup_bytes = fs::read(&backup).unwrap();
        let mut command = relaunch_command(&image, &backup, u32::MAX).unwrap();
        let mut helper = command.env("PATH", format!("{}:/usr/bin:/bin", tools.display()))
            .env("TEST_STARTED", &started).env("TEST_TERM", &term).env("TEST_RECOVERED", &recovered)
            .spawn().unwrap();
        drop(command);
        let deadline = Instant::now() + Duration::from_secs(12);
        let mut result = None;
        while Instant::now() < deadline {
            if let Some(status) = helper.try_wait().unwrap() { result = Some(status); break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        let fixture_pid: i32 = fs::read_to_string(&started).unwrap().parse().unwrap();
        let fixture_cmd = fs::read(format!("/proc/{fixture_pid}/cmdline")).unwrap();
        assert!(fixture_cmd.split(|b| *b == 0).any(|arg| arg == image.as_os_str().as_encoded_bytes()), "cleanup must own this exact fixture");
        let terminated_cooperatively = term.exists();
        let still_alive = unsafe { libc::kill(fixture_pid, 0) } == 0;
        let lock_released = transaction_lock(&image).is_ok();
        let left_images_untouched = fs::read(&marker).unwrap() == next
            && fs::read(&previous_marker).unwrap() == previous
            && fs::read(&backup).unwrap() == backup_bytes
            && !recovered.exists() && !failed_marker.exists();
        // Only this synthetic fixture is killed for test cleanup. Production
        // must not turn a startup timeout into SIGKILL of a possibly busy app.
        unsafe { libc::kill(fixture_pid, libc::SIGKILL); }
        if result.is_none() { let _ = helper.wait(); }
        fs::remove_dir_all(dir).unwrap();
        assert!(result.is_some(), "readiness timeout must not wait forever or retain its transaction lock");
        assert!(!result.unwrap().success(), "unresolved startup is not successful recovery");
        assert!(terminated_cooperatively && still_alive, "SIGTERM must be observed without forcing exit");
        assert!(lock_released, "later updates must remain possible");
        assert!(left_images_untouched, "a live child forbids rollback or blacklisting");
    }

    #[test]
    fn failed_post_exit_launch_recovers_the_previous_image() {
        let dir = std::env::temp_dir().join(format!("apparent-relaunch-{}", crate::shared::ids::make_bundle_id()));
        fs::create_dir(&dir).unwrap();
        let image = dir.join("new.AppImage");
        let backup = dir.join("previous.AppImage");
        let marker = dir.join("recovered");
        let sidecar = image.with_extension("AppImage.apparent-update.json");
        let previous_sidecar = backup.with_file_name("previous.AppImage.apparent-update.json");
        let failed_sidecar = image.with_file_name(".new.AppImage.failed-update.json");
        fs::write(&image, b"#!/bin/sh\nexit 1\n").unwrap();
        fs::write(&backup, b"#!/bin/sh\nprintf recovered > \"$TEST_RECOVERY_MARKER\"\n").unwrap();
        fs::write(&sidecar, br#"{"channel":"stable","id":"v99.0.0"}"#).unwrap();
        fs::write(&previous_sidecar, br#"{"channel":"stable","id":"v1.0.0"}"#).unwrap();
        for path in [&image, &backup] {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(path, permissions).unwrap();
        }
        assert!(relaunch_command(&image, &backup, u32::MAX)
            .unwrap().env("TEST_RECOVERY_MARKER", &marker).status().unwrap().success());
        assert_eq!(fs::read(&marker).unwrap(), b"recovered");
        assert_eq!(fs::read(&image).unwrap(), b"#!/bin/sh\nprintf recovered > \"$TEST_RECOVERY_MARKER\"\n");
        assert_eq!(installed_id(&image), Some(("stable".into(), "v1.0.0".into())), "rollback must restore the old version marker");
        let bad = Candidate { id: "v99.0.0".into(), channel: "stable", url: String::new(), sha256: String::new() };
        let next = Candidate { id: "v100.0.0".into(), channel: "stable", url: String::new(), sha256: String::new() };
        assert!(previously_failed(&failed_sidecar, &bad), "the broken release must not be reinstalled on startup");
        assert!(!previously_failed(&failed_sidecar, &next), "a newer release must remain eligible");
        fs::remove_file(&marker).unwrap();
        assert!(Command::new(&image).env("TEST_RECOVERY_MARKER", &marker).status().unwrap().success());
        assert_eq!(fs::read(&marker).unwrap(), b"recovered", "the desktop launcher must work on the next start");
        fs::write(&backup, fs::read(&image).unwrap()).unwrap();
        let mut permissions = fs::metadata(&backup).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&backup, permissions).unwrap();
        fs::remove_file(&image).unwrap();
        fs::remove_file(&marker).unwrap();
        assert!(relaunch_command(&image, &backup, u32::MAX)
            .unwrap().env("TEST_RECOVERY_MARKER", &marker).status().unwrap().success());
        assert_eq!(fs::read(&marker).unwrap(), b"recovered");
        fs::write(&backup, fs::read(&image).unwrap()).unwrap();
        let mut permissions = fs::metadata(&backup).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&backup, permissions).unwrap();
        fs::remove_file(&marker).unwrap();
        fs::write(&image, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&image).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&image, permissions).unwrap();
        assert!(relaunch_command(&image, &backup, u32::MAX)
            .unwrap().env("TEST_RECOVERY_MARKER", &marker).status().unwrap().success());
        assert_eq!(fs::read(&marker).unwrap(), b"recovered", "a clean early exit without a running app still needs recovery");
        fs::write(&backup, fs::read(&image).unwrap()).unwrap();
        let mut permissions = fs::metadata(&backup).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&backup, permissions).unwrap();
        fs::write(&image, b"#!/bin/sh\nsleep 5\nexit 1\n").unwrap();
        let mut permissions = fs::metadata(&image).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&image, permissions).unwrap();
        fs::remove_file(&marker).unwrap();
        assert!(relaunch_command(&image, &backup, u32::MAX)
            .unwrap().env("TEST_RECOVERY_MARKER", &marker).status().unwrap().success());
        assert_eq!(fs::read(&marker).unwrap(), b"recovered", "a slow failure before GPUI readiness needs recovery");
        fs::write(&backup, fs::read(&image).unwrap()).unwrap();
        let mut permissions = fs::metadata(&backup).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&backup, permissions).unwrap();
        fs::write(&image, b"#!/bin/sh\ntouch \"$APPARENT_UPDATE_READY\"\nsleep 5\nexit 1\n").unwrap();
        let mut permissions = fs::metadata(&image).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&image, permissions).unwrap();
        fs::remove_file(&marker).unwrap();
        assert!(relaunch_command(&image, &backup, u32::MAX)
            .unwrap().env("TEST_RECOVERY_MARKER", &marker).status().unwrap().success());
        std::thread::sleep(Duration::from_secs(6));
        assert!(!marker.exists(), "a failure after GPUI readiness must not resurrect the old image");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn update_identity_reads_its_sidecar() {
        let dir = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        fs::create_dir(&dir).unwrap();
        let image = dir.join("Apparent.AppImage");
        fs::write(image.with_extension("AppImage.apparent-update.json"), br#"{"channel":"main","id":"new"}"#).unwrap();
        assert_eq!(installed_id(&image), Some(("main".into(), "new".into())));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bad_download_cannot_replace_the_image_and_exchange_preserves_the_previous_one() {
        let dir = std::env::temp_dir().join(format!("apparent-update-test-{}", crate::shared::ids::make_bundle_id()));
        fs::create_dir(&dir).unwrap();
        let image = dir.join("Apparent.AppImage");
        let stage = dir.join("download");
        fs::write(&image, b"old image").unwrap();
        fs::write(&stage, b"new image").unwrap();
        assert!(verify_download(&stage, &"0".repeat(64)).is_err());
        assert_eq!(fs::read(&image).unwrap(), b"old image");
        let hash = format!("{:x}", Sha256::digest(b"new image"));
        verify_download(&stage, &hash).unwrap();
        exchange(&stage, &image).unwrap();
        assert_eq!(fs::read(&image).unwrap(), b"new image");
        assert_eq!(fs::read(&stage).unwrap(), b"old image");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stable_never_downgrades_without_a_channel_change() {
        let candidate = Candidate { id: "v0.3.13".into(), channel: "stable", url: String::new(), sha256: String::new() };
        assert!(!newer(&candidate, Some(("stable".into(), "v0.3.14".into()))));
        assert!(!newer(&candidate, Some(("stable".into(), "v0.3.13".into()))));
        assert!(newer(&candidate, Some(("stable".into(), "v0.3.12".into()))));
        assert!(newer(&candidate, Some(("main".into(), "a".repeat(40)))));
    }

    #[test]
    fn channel_command_preserves_other_config_and_rejects_invalid_choices() {
        let dir = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        fs::create_dir(&dir).unwrap();
        let path = dir.join("config.json");
        fs::write(&path, br#"{"ingestToken":"private","autoSend":false}"#).unwrap();
        set_channel_in(&path, "main").unwrap();
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["updateChannel"], "main");
        assert_eq!(value["ingestToken"], "private");
        assert_eq!(value["autoSend"], false);
        let before = fs::read(&path).unwrap();
        assert!(set_channel_in(&path, "edge").is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_records_the_running_version_without_touching_preferences() {
        let dir = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        fs::create_dir(&dir).unwrap();
        let path = dir.join("config.json");
        fs::write(&path, br#"{"updateChannel":"main","ingestToken":"private"}"#).unwrap();
        write_runtime_in(&path, 42, 99).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["running"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["running"]["pid"], 42);
        assert_eq!(value["running"]["startTicks"], 99);
        assert_eq!(value["updateChannel"], "main");
        assert_eq!(value["ingestToken"], "private");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn status_only_reports_a_live_process_with_the_recorded_start_time() {
        let dir = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        fs::create_dir(&dir).unwrap();
        let path = dir.join("config.json");
        let pid = std::process::id();
        let ticks = process_ticks(pid).unwrap();
        write_runtime_in(&path, pid, ticks).unwrap();
        assert_eq!(status_at(&path).unwrap()["running"]["version"], env!("CARGO_PKG_VERSION"));
        write_runtime_in(&path, pid, ticks + 1).unwrap();
        assert!(status_at(&path).unwrap()["running"].is_null());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn exchanged_appimage_still_identifies_its_running_process() {
        assert!(same_executable(Path::new("/home/user/Apparent.AppImage"), "/home/user/Apparent.AppImage"));
        assert!(same_executable(Path::new("/home/user/Apparent.AppImage (deleted)"), "/home/user/Apparent.AppImage"));
        assert!(!same_executable(Path::new("/home/user/Other.AppImage (deleted)"), "/home/user/Apparent.AppImage"));
    }
}
