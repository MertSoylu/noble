//! Updates: checking GitHub for the latest release (in the background, once a day)
//! and `noble update` — downloads the ready-made binary and replaces the running one.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::event::{AppEvent, Tx};

const LATEST_URL: &str = "https://api.github.com/repos/MertSoylu/noble/releases/latest";
pub const RELEASES_URL: &str = "https://github.com/MertSoylu/noble/releases/latest";
/// Interval between checks (seconds).
pub const CHECK_INTERVAL: i64 = 24 * 3600;
/// Env var that turns the check off (for tests and packagers).
pub const DISABLE_ENV: &str = "NOBLE_NO_UPDATE_CHECK";

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// "v1.2.3", "1.2.3-beta.1" → (1, 2, 3). The pre-release suffix is ignored.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?);
    parts.next().is_none().then_some(v)
}

/// Is `latest` newer than `current` (an unreadable version never counts as newer).
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// Is the check enabled in this install: `noble-dev` is built from source, the env var turns it off.
pub fn check_allowed() -> bool {
    !crate::util::is_dev_build() && std::env::var_os(DISABLE_ENV).is_none()
}

pub struct Release {
    /// Without the leading "v": "1.2.0".
    pub version: String,
    /// (file name, download URL)
    pub assets: Vec<(String, String)>,
}

fn agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .user_agent(concat!("noble/", env!("CARGO_PKG_VERSION")))
        .build();
    ureq::Agent::new_with_config(config)
}

fn net_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "timeout".into(),
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => "offline".into(),
        e => format!("network error ({e})"),
    }
}

/// Version and files from the GitHub API response.
pub fn parse_release(v: &Value) -> Option<Release> {
    let tag = v.get("tag_name")?.as_str()?;
    parse_version(tag)?;
    let assets = v
        .get("assets")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    let name = a.get("name")?.as_str()?;
                    let url = a.get("browser_download_url")?.as_str()?;
                    Some((name.to_string(), url.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Release { version: tag.trim_start_matches(['v', 'V']).to_string(), assets })
}

/// Reads the latest published release from GitHub.
pub fn fetch_latest() -> Result<Release, String> {
    let mut resp = agent(Duration::from_secs(10))
        .get(LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(net_error)?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    match status {
        200 => {}
        403 | 429 => return Err("GitHub rate limit, try again later".into()),
        404 => return Err("no release published yet".into()),
        s => return Err(format!("HTTP {s}")),
    }
    let v: Value = serde_json::from_str(&text).map_err(|_| "unexpected response".to_string())?;
    parse_release(&v).ok_or_else(|| "unexpected response".into())
}

/// Asks for the latest release in the background; the result arrives as `AppEvent::Update`.
pub fn spawn_check(tx: Tx) {
    let _ = std::thread::Builder::new().name("update".into()).spawn(move || {
        let _ = tx.send(AppEvent::Update(fetch_latest().map(|r| r.version)));
    });
}

/// Name of the ready-made archive for this platform (see `release.yml`).
pub fn asset_name() -> Option<&'static str> {
    asset_name_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// Archive name for an OS / CPU pair as `std::env::consts` names them.
fn asset_name_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("noble-windows-x86_64.zip"),
        ("linux", "x86_64") => Some("noble-linux-x86_64.tar.gz"),
        ("linux", "aarch64") => Some("noble-linux-aarch64.tar.gz"),
        _ => None,
    }
}

fn binary_name() -> &'static str {
    if cfg!(windows) { "noble.exe" } else { "noble" }
}

/// The old binary left by an update: `noble.exe` → `noble.exe.old`.
fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    exe.with_file_name(name)
}

/// Deletes the old binary left by a previous update (a running binary cannot be
/// deleted on Windows; it is renamed instead and cleaned up on the next start).
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        cleanup_old_at(&exe);
    }
}

fn cleanup_old_at(exe: &Path) {
    for p in old_paths(exe) {
        let _ = std::fs::remove_file(p);
    }
}

/// `noble.old`, then `noble.old2` … `noble.old9` (used when an earlier one cannot be deleted).
fn old_paths(exe: &Path) -> impl Iterator<Item = PathBuf> {
    let first = old_path(exe);
    (1..=9).map(move |i| {
        if i == 1 {
            return first.clone();
        }
        let mut name = first.as_os_str().to_os_string();
        name.push(i.to_string());
        PathBuf::from(name)
    })
}

/// The system `tar` (Windows 10+ `tar.exe` opens zips too).
fn tar_program() -> PathBuf {
    // Windows: Git Bash's GNU tar cannot open zips, so System32's bsdtar is preferred.
    // Linux: whatever `tar` is on PATH.
    std::env::var_os("SystemRoot")
        .filter(|_| cfg!(windows))
        .map(|r| PathBuf::from(r).join("System32").join("tar.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("tar"))
}

/// Extracts the archive with `tar`.
fn extract(tar: &Path, archive: &Path, dest: &Path) -> Result<(), String> {
    let out = std::process::Command::new(tar)
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .output()
        .map_err(|e| format!("could not run tar: {e}"))?;
    if !out.status.success() {
        return Err(format!("could not unpack the archive: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

fn download(url: &str, to: &Path) -> Result<(), String> {
    let resp = agent(Duration::from_secs(300)).get(url).call().map_err(net_error)?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(format!("download failed: HTTP {status}"));
    }
    let mut file = std::fs::File::create(to).map_err(|e| format!("could not write {}: {e}", to.display()))?;
    std::io::copy(&mut resp.into_body().into_reader(), &mut file).map_err(|e| format!("download failed: {e}"))?;
    file.flush().map_err(|e| e.to_string())
}

/// A free name to move the running binary aside to. An earlier `.old` may not be deletable (on Windows
/// it can still be running in another NOBLE window), so a numbered one is used then.
fn free_old_path(exe: &Path) -> Option<PathBuf> {
    old_paths(exe).find(|p| {
        let _ = std::fs::remove_file(p);
        std::fs::symlink_metadata(p).is_err()
    })
}

/// Puts the new binary in place of the running one. The copy (slow, may fail half way) goes to a
/// staging file next to it first; then the running binary is moved aside (it cannot be overwritten on
/// Windows but it can be renamed) and the staged one renamed into place. Any failure leaves the
/// installed binary where it was.
fn replace_exe(new: &Path, exe: &Path) -> Result<(), String> {
    let mut staged = exe.as_os_str().to_os_string();
    staged.push(".new");
    let staged = PathBuf::from(staged);
    let _ = std::fs::remove_file(&staged);
    if let Err(e) = std::fs::copy(new, &staged) {
        let _ = std::fs::remove_file(&staged);
        return Err(format!("could not write the new binary next to {}: {e}", exe.display()));
    }
    // Unix: make it executable. Windows: no mode bits, the `.exe` name is enough.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755));
    }
    let aside = free_old_path(exe)
        .ok_or_else(|| format!("could not move {} aside: no free .old name", exe.display()))
        .and_then(|old| {
            std::fs::rename(exe, &old).map_err(|e| format!("could not move {} aside: {e}", exe.display()))?;
            Ok(old)
        });
    let old = match aside {
        Ok(old) => old,
        Err(e) => {
            let _ = std::fs::remove_file(&staged);
            return Err(e);
        }
    };
    if let Err(e) = std::fs::rename(&staged, exe) {
        let _ = std::fs::remove_file(&staged);
        return match std::fs::rename(&old, exe) {
            Ok(()) => Err(format!("could not install the new binary: {e}")),
            Err(e2) => Err(format!(
                "could not install the new binary: {e}; the previous one is at {} (could not restore: {e2})",
                old.display()
            )),
        };
    }
    // Linux: it can be deleted right away. Windows: the running binary cannot, `cleanup_old` does it
    // on the next start.
    let _ = std::fs::remove_file(&old);
    Ok(())
}

/// Downloads the release's archive for this platform and puts it in place of `exe`.
pub fn install(release: &Release, exe: &Path) -> Result<(), String> {
    install_with(release, exe, &download, &tar_program())
}

/// `install` with the downloader and `tar` passed in (tests use a local fetcher, never the network).
fn install_with(
    release: &Release,
    exe: &Path,
    fetch: &dyn Fn(&str, &Path) -> Result<(), String>,
    tar: &Path,
) -> Result<(), String> {
    let name = asset_name().ok_or("no prebuilt binary for this platform; update with `cargo install noble`")?;
    let url = release
        .assets
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, u)| u.clone())
        .ok_or_else(|| format!("release {} has no {name}", release.version))?;
    // Unique per call, not only per process (parallel tests install at the same time).
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let work = std::env::temp_dir().join(format!("noble-update-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let result = (|| {
        let archive = work.join(name);
        println!("Downloading {name}…");
        fetch(&url, &archive)?;
        extract(tar, &archive, &work)?;
        let new = work.join(binary_name());
        if !new.is_file() {
            return Err(format!("{} not found in the archive", binary_name()));
        }
        if new.metadata().map(|m| m.len()).unwrap_or(0) == 0 {
            return Err(format!("{} in the archive is empty", binary_name()));
        }
        replace_exe(&new, exe)
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// `noble update [--check]`: returns an exit code.
pub fn run_cli(args: &[String]) -> i32 {
    let check_only = match args.first().map(String::as_str) {
        None => false,
        Some("--check") => true,
        Some("-h" | "--help") => {
            println!(
                "USAGE: noble update [--check]\n\n\
                 Downloads the latest release from GitHub and replaces this binary.\n  \
                   --check   only report whether a newer version exists"
            );
            return 0;
        }
        Some(other) => {
            eprintln!("unknown argument '{other}' (try noble update --help)");
            return 2;
        }
    };
    cleanup_old();
    println!("noble {} · checking for updates…", current());
    let release = match fetch_latest() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: could not check for updates: {e}");
            return 1;
        }
    };
    if !is_newer(&release.version, current()) {
        println!("noble {} is up to date.", current());
        return 0;
    }
    println!("noble {} is available.", release.version);
    if check_only {
        println!("Run `noble update` to install it.");
        return 0;
    }
    if crate::util::is_dev_build() {
        eprintln!("noble-dev is built from source; update it with install.cmd / install.sh instead.");
        return 1;
    }
    let exe = match std::env::current_exe() {
        Ok(p) => std::fs::canonicalize(&p).unwrap_or(p),
        Err(e) => {
            eprintln!("error: could not locate this binary: {e}");
            return 1;
        }
    };
    match install(&release, &exe) {
        Ok(()) => {
            println!("Updated noble {} → {}. Restart NOBLE to use the new version.", current(), release.version);
            0
        }
        Err(e) => {
            eprintln!("error: {e}\nYou can also download it from {RELEASES_URL}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.10.0-beta.1"), Some((1, 10, 0)));
        assert_eq!(parse_version("2"), Some((2, 0, 0)));
        assert_eq!(parse_version("latest"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert!(is_newer("v1.10.0", "1.9.9"));
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("garbage", "1.0.0"));
    }

    #[test]
    fn release_payload_is_parsed() {
        let v: Value = serde_json::json!({
            "tag_name": "v1.1.0",
            "assets": [
                { "name": "noble-windows-x86_64.zip", "browser_download_url": "https://example.com/w.zip" },
                { "name": "noble-linux-x86_64.tar.gz", "browser_download_url": "https://example.com/l.tgz" }
            ]
        });
        let r = parse_release(&v).unwrap();
        assert_eq!(r.version, "1.1.0");
        assert_eq!(r.assets.len(), 2);
        assert!(parse_release(&serde_json::json!({ "tag_name": "nightly" })).is_none());
    }

    /// Downloads the real latest release over a temporary "installed" binary (needs network).
    /// `cargo test --lib update::tests::installs_latest_release -- --ignored`
    #[test]
    #[ignore]
    fn installs_latest_release() {
        let dir = std::env::temp_dir().join(format!("noble-install-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(binary_name());
        std::fs::write(&exe, b"old").unwrap();
        let release = fetch_latest().expect("latest release");
        install(&release, &exe).expect("install");
        let out = std::process::Command::new(&exe).arg("--version").output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains(&release.version), "{text}");
        assert!(!old_path(&exe).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A scratch directory holding a fake "installed" binary.
    fn scratch(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("noble-update-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(binary_name());
        std::fs::write(&exe, b"installed").unwrap();
        (dir, exe)
    }

    fn release_for_this_platform() -> Option<Release> {
        let name = asset_name()?;
        Some(Release { version: "9.9.9".into(), assets: vec![(name.into(), "http://fake/asset".into())] })
    }

    /// A real archive for this platform holding `binary_name()` with `content`, built with the system tar
    /// (Windows 10+ `tar.exe` writes zips with `-a`; GNU tar on Linux writes tar.gz with `-z`).
    fn make_archive(dir: &Path, content: &[u8]) -> Option<Vec<u8>> {
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join(binary_name()), content).unwrap();
        let out = dir.join(asset_name()?);
        let mode = if cfg!(windows) { "-acf" } else { "-czf" };
        let ok = std::process::Command::new(tar_program())
            .arg(mode)
            .arg(&out)
            .arg("-C")
            .arg(&src)
            .arg(binary_name())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        ok.then(|| std::fs::read(&out).unwrap())
    }

    /// A fetcher that "downloads" fixed bytes, never touching the network.
    fn serve(bytes: Vec<u8>) -> impl Fn(&str, &Path) -> Result<(), String> {
        move |_url, to| std::fs::write(to, &bytes).map_err(|e| e.to_string())
    }

    fn assert_intact(exe: &Path) {
        assert_eq!(std::fs::read(exe).unwrap(), b"installed", "the installed binary was damaged");
    }

    #[test]
    fn corrupt_download_keeps_the_installed_binary() {
        let Some(release) = release_for_this_platform() else { return }; // no prebuilt archive here
        let (dir, exe) = scratch("corrupt");
        let err = install_with(&release, &exe, &serve(b"<html>captive portal</html>".to_vec()), &tar_program());
        assert!(err.is_err());
        assert_intact(&exe);
        // A valid archive whose binary is empty (a broken release) is refused too.
        if let Some(empty) = make_archive(&dir, b"") {
            assert!(install_with(&release, &exe, &serve(empty), &tar_program()).is_err());
            assert_intact(&exe);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_tar_is_reported_and_keeps_the_installed_binary() {
        let Some(release) = release_for_this_platform() else { return };
        let (dir, exe) = scratch("notar");
        let archive = make_archive(&dir, b"new").unwrap_or_default();
        let err = install_with(&release, &exe, &serve(archive), Path::new("noble-no-such-tar")).unwrap_err();
        assert!(err.contains("tar"), "{err}");
        assert_intact(&exe);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undeletable_old_binary_does_not_block_the_update() {
        let Some(release) = release_for_this_platform() else { return };
        let (dir, exe) = scratch("oldlocked");
        let Some(archive) = make_archive(&dir, b"new") else { return }; // no tar on this machine
        // A directory cannot be removed with remove_file nor replaced by a rename, on either platform —
        // like a previous `noble.exe.old` still running in another NOBLE window on Windows.
        std::fs::create_dir_all(old_path(&exe).join("busy")).unwrap();
        install_with(&release, &exe, &serve(archive), &tar_program()).expect("update");
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        let mut staged = exe.as_os_str().to_os_string();
        staged.push(".new");
        assert!(!Path::new(&staged).exists(), "staging file left behind");
        cleanup_old_at(&exe);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_only_install_dir_keeps_the_installed_binary() {
        let Some(release) = release_for_this_platform() else { return };
        let (dir, exe) = scratch("readonly");
        let Some(archive) = make_archive(&dir, b"new") else { return };
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe_ro = bin.join(binary_name());
        std::fs::rename(&exe, &exe_ro).unwrap();
        // Unix: a directory without write permission. Windows has no such mode bit on directories (a
        // read-only flag is ignored there), so the case is only exercised on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o555)).unwrap();
            // root ignores permissions (as in containers): nothing to test then.
            if std::fs::write(bin.join("probe"), b"").is_ok() {
                std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            let err = install_with(&release, &exe_ro, &serve(archive), &tar_program()).unwrap_err();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(!err.is_empty());
            assert_intact(&exe_ro);
            assert_eq!(std::fs::read_dir(&bin).unwrap().count(), 1, "leftover files next to the binary");
        }
        #[cfg(not(unix))]
        let _ = (archive, release);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A local server that promises more bytes than it sends, then hangs up.
    #[test]
    fn partial_download_is_an_error() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nConnection: close\r\n\r\npartial");
        });
        let dir = std::env::temp_dir().join(format!("noble-update-partial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let to = dir.join("archive");
        let res = download(&format!("http://127.0.0.1:{port}/asset"), &to);
        server.join().unwrap();
        assert!(res.is_err(), "a truncated download was accepted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn asset_names_match_release_workflow() {
        assert_eq!(asset_name_for("windows", "x86_64"), Some("noble-windows-x86_64.zip"));
        assert_eq!(asset_name_for("linux", "x86_64"), Some("noble-linux-x86_64.tar.gz"));
        assert_eq!(asset_name_for("linux", "aarch64"), Some("noble-linux-aarch64.tar.gz"));
        assert_eq!(asset_name_for("macos", "aarch64"), None);
        assert_eq!(asset_name(), asset_name_for(std::env::consts::OS, std::env::consts::ARCH));
        // Every archive release.yml builds is the one `noble update` asks for on that target.
        let yml =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/.github/workflows/release.yml")).unwrap();
        let field = |line: &str, key: &str| line.trim().strip_prefix(key).map(|v| v.trim().to_string());
        let targets: Vec<String> =
            yml.lines().filter_map(|l| field(l, "target:")).filter(|t| !t.contains('$')).collect();
        let archives: Vec<String> = yml.lines().filter_map(|l| field(l, "archive:")).collect();
        assert_eq!(targets.len(), 3, "{targets:?}");
        assert_eq!(targets.len(), archives.len());
        for (target, archive) in targets.iter().zip(&archives) {
            let arch = target.split('-').next().unwrap();
            let os = if target.contains("windows") { "windows" } else { "linux" };
            assert_eq!(asset_name_for(os, arch), Some(archive.as_str()), "{target}");
        }
    }

    #[test]
    fn old_binary_sits_next_to_the_exe() {
        let p = old_path(Path::new("/usr/bin/noble"));
        assert_eq!(p, Path::new("/usr/bin/noble.old"));
    }
}
