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
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("noble-windows-x86_64.zip")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("noble-linux-x86_64.tar.gz")
    } else {
        None
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
        let _ = std::fs::remove_file(old_path(&exe));
    }
}

/// Extracts the archive with the system `tar` (Windows 10+ `tar.exe` opens zips too).
fn extract(archive: &Path, dest: &Path) -> Result<(), String> {
    // Git Bash's GNU tar cannot open zips: use the system one on Windows.
    let tar = std::env::var_os("SystemRoot")
        .filter(|_| cfg!(windows))
        .map(|r| PathBuf::from(r).join("System32").join("tar.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("tar"));
    let out = std::process::Command::new(&tar)
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

/// Puts the new binary in place of the running one. The running binary is moved
/// aside first (it cannot be overwritten on Windows but it can be renamed); it
/// is put back if the copy fails.
fn replace_exe(new: &Path, exe: &Path) -> Result<(), String> {
    let old = old_path(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| format!("could not move {} aside: {e}", exe.display()))?;
    if let Err(e) = std::fs::copy(new, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(format!("could not install the new binary: {e}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o755));
    }
    // On Linux it can be deleted right away; on Windows on the next start.
    let _ = std::fs::remove_file(&old);
    Ok(())
}

/// Downloads the release's archive for this platform and puts it in place of `exe`.
pub fn install(release: &Release, exe: &Path) -> Result<(), String> {
    let name = asset_name().ok_or("no prebuilt binary for this platform; update with `cargo install noble`")?;
    let url = release
        .assets
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, u)| u.clone())
        .ok_or_else(|| format!("release {} has no {name}", release.version))?;
    let work = std::env::temp_dir().join(format!("noble-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let result = (|| {
        let archive = work.join(name);
        println!("Downloading {name}…");
        download(&url, &archive)?;
        extract(&archive, &work)?;
        let new = work.join(binary_name());
        if !new.is_file() {
            return Err(format!("{} not found in the archive", binary_name()));
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
        eprintln!("noble-dev is built from source; update it with install.cmd instead.");
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

    #[test]
    fn old_binary_sits_next_to_the_exe() {
        let p = old_path(Path::new("/usr/bin/noble"));
        assert_eq!(p, Path::new("/usr/bin/noble.old"));
    }
}
