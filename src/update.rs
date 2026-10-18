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

/// Hosts a release's files come from: GitHub, and the storage its download links redirect to.
const DOWNLOAD_HOSTS: [&str; 3] =
    ["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];

/// Largest archive a download may be (a NOBLE binary is a few MB) and largest checksum file (a few lines).
const MAX_ARCHIVE: u64 = 256 * 1024 * 1024;
const MAX_SUMS: u64 = 64 * 1024;

/// An agent for the GitHub API (https only).
fn agent(timeout: Duration) -> ureq::Agent {
    agent_with(timeout, true)
}

/// An agent; `https_only` is off only for the local test server of `partial_download_is_an_error`.
fn agent_with(timeout: Duration, https_only: bool) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .https_only(https_only)
        .http_status_as_error(false)
        .user_agent(concat!("noble/", env!("CARGO_PKG_VERSION")))
        .build();
    ureq::Agent::new_with_config(config)
}

/// Is `url` an https address on one of `DOWNLOAD_HOSTS`, with no user info or port? The asset list comes from
/// the API response, so each address in it is checked before anything is fetched from it.
pub fn is_release_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    DOWNLOAD_HOSTS.iter().any(|known| host.eq_ignore_ascii_case(known))
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
        ("windows", "aarch64") => Some("noble-windows-aarch64.zip"),
        ("linux", "x86_64") => Some("noble-linux-x86_64.tar.gz"),
        ("linux", "aarch64") => Some("noble-linux-aarch64.tar.gz"),
        ("macos", "aarch64") => Some("noble-macos-aarch64.tar.gz"),
        ("macos", "x86_64") => Some("noble-macos-x86_64.tar.gz"),
        _ => None,
    }
}

/// Name of the checksum file `release.yml` publishes next to each archive: `<archive>.sha256`.
pub fn checksum_name(archive: &str) -> String {
    format!("{archive}.sha256")
}

/// The SHA-256 (lower-case hex) for `name` from a `sha256sum`-style file: `<hex>  <name>` or
/// `<hex> *<name>` lines (several files may be listed); a line holding only the hash counts for any name.
pub fn parse_checksum(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next().map(|f| f.trim_start_matches('*'));
        let valid = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        (valid && file.is_none_or(|f| f == name)).then(|| hash.to_ascii_lowercase())
    })
}

/// SHA-256 of a file, lower-case hex.
fn sha256_file(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let read_err = |e: std::io::Error| format!("could not read {}: {e}", path.display());
    let mut file = std::fs::File::open(path).map_err(read_err)?;
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(read_err)?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    Ok(ctx.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect())
}

/// Checks the downloaded archive against the release's checksum file, before anything is unpacked.
fn verify(archive: &Path, name: &str, sums: &str) -> Result<(), String> {
    let expected = parse_checksum(sums, name).ok_or_else(|| format!("the checksum file has no SHA-256 for {name}"))?;
    let actual = sha256_file(archive)?;
    if actual != expected {
        return Err(format!(
            "checksum mismatch for {name} (expected {expected}, got {actual}): the download is corrupted or \
             incomplete, nothing was installed"
        ));
    }
    Ok(())
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
    // Linux and macOS: whatever `tar` is on PATH (bsdtar on macOS, which reads .tar.gz as well).
    std::env::var_os("SystemRoot")
        .filter(|_| cfg!(windows))
        .map(|r| PathBuf::from(r).join("System32").join("tar.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("tar"))
}

/// Checks what `tar` would write before it writes anything: `names` is `tar -tf`, `verbose` is `tar -tvf` (the
/// same entries in the same order). Refused: an absolute name, a `..` part (both would leave the work folder),
/// and a link (a symlink or a hard link can point anywhere). Only regular files and folders are unpacked.
fn check_listing(names: &str, verbose: &str) -> Result<(), String> {
    let names: Vec<&str> = names.lines().collect();
    let kinds: Vec<&str> = verbose.lines().collect();
    if names.len() != kinds.len() {
        return Err("the archive listing does not match its entries, nothing was unpacked".into());
    }
    for (name, line) in names.iter().zip(&kinds) {
        let absolute = name.starts_with(['/', '\\']) || name.as_bytes().get(1) == Some(&b':');
        let escapes = name.split(['/', '\\']).any(|part| part == "..");
        let plain = matches!(line.as_bytes().first(), Some(b'-' | b'd')) && !line.contains(" link to ");
        if absolute || escapes || !plain {
            return Err(format!("the archive has an unsafe entry ({name}), nothing was unpacked"));
        }
    }
    Ok(())
}

/// The entries of an archive, from `tar` (names, then verbose lines for the entry types).
fn list_archive(tar: &Path, archive: &Path) -> Result<(String, String), String> {
    let list = |flag: &str| -> Result<String, String> {
        let out = std::process::Command::new(tar)
            .arg(flag)
            .arg(archive)
            .output()
            .map_err(|e| format!("could not run tar: {e}"))?;
        if !out.status.success() {
            return Err(format!("could not read the archive: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    Ok((list("-tf")?, list("-tvf")?))
}

/// Unpacks the archive with `tar` into `dest`, after `check_listing` has passed every entry.
fn extract(tar: &Path, archive: &Path, dest: &Path) -> Result<(), String> {
    let (names, verbose) = list_archive(tar, archive)?;
    check_listing(&names, &verbose)?;
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

/// Copies at most `max` bytes from `from` to `to`. More than that is refused (the copy stops at `max + 1`),
/// so a runaway download cannot fill the disk.
fn copy_capped(from: &mut impl std::io::Read, to: &mut impl std::io::Write, max: u64) -> Result<(), String> {
    let mut limited = std::io::Read::take(from, max + 1);
    let copied = std::io::copy(&mut limited, to).map_err(|e| format!("download failed: {e}"))?;
    if copied > max {
        return Err(format!("the download is larger than {max} bytes, nothing was installed"));
    }
    Ok(())
}

/// Downloads `url` into `to`, at most `max` bytes.
fn download(url: &str, to: &Path, max: u64) -> Result<(), String> {
    download_via(&agent(Duration::from_secs(300)), url, to, max)
}

fn download_via(agent: &ureq::Agent, url: &str, to: &Path, max: u64) -> Result<(), String> {
    let resp = agent.get(url).call().map_err(net_error)?;
    let status = resp.status().as_u16();
    if status != 200 {
        return Err(format!("download failed: HTTP {status}"));
    }
    let mut file = std::fs::File::create(to).map_err(|e| format!("could not write {}: {e}", to.display()))?;
    copy_capped(&mut resp.into_body().into_reader(), &mut file, max)?;
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
/// staging file next to it first, so the rename below stays on one file system. Unix: one rename replaces
/// the binary in a single step (the running process keeps the old inode), so a kill at any moment leaves
/// either the old or the new binary. Windows: the running binary cannot be overwritten but it can be
/// renamed, so it is moved aside first and the staged one renamed into place. Any failure leaves the
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
    if cfg!(unix) {
        // Unix: the binary is replaced in one step; the old one needs no clean-up.
        return std::fs::rename(&staged, exe).map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            format!("could not install the new binary: {e}")
        });
    }
    // Windows: move the running binary aside first (`.old`, `.old2` …), then rename the staged one into place.
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
    // Windows: the running binary cannot be deleted; `cleanup_old` does it on the next start.
    Ok(())
}

/// A new, empty directory under the temp dir that only this call uses. The temp dir is shared with other
/// users on Linux, so a name that already exists is never reused (someone else may own it and have planted
/// symlinks in it): the directory is created fresh, with a hard-to-guess name, and on Unix readable by
/// the owner only. Windows: the per-user `%TEMP%` is not shared, the ACL comes from it.
fn private_work_dir() -> Result<PathBuf, String> {
    // Unique per call, not only per process (parallel tests install at the same time).
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let mut last = String::new();
    for _ in 0..16 {
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let salt = {
            use std::hash::{BuildHasher, Hasher};
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(nanos);
            h.write_u32(seq);
            h.finish()
        };
        let work = std::env::temp_dir().join(format!("noble-update-{}-{seq}-{salt:016x}", std::process::id()));
        #[allow(unused_mut)]
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&work) {
            Ok(()) => return Ok(work),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = e.to_string(),
            Err(e) => return Err(format!("could not create a work directory in {}: {e}", work.display())),
        }
    }
    Err(format!("could not create a work directory: {last}"))
}

/// Downloads the release's archive for this platform and puts it in place of `exe`.
pub fn install(release: &Release, exe: &Path) -> Result<(), String> {
    install_with(release, exe, &download, &tar_program())
}

/// `install` with the downloader and `tar` passed in (tests use a local fetcher, never the network).
/// `fetch` gets the address, the file to write and the most bytes it may write.
fn install_with(
    release: &Release,
    exe: &Path,
    fetch: &dyn Fn(&str, &Path, u64) -> Result<(), String>,
    tar: &Path,
) -> Result<(), String> {
    let name = asset_name().ok_or("no prebuilt binary for this platform; update with `cargo install noble`")?;
    let url = release
        .assets
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, u)| u.clone())
        .ok_or_else(|| format!("release {} has no {name}", release.version))?;
    // Releases from before checksums were published cannot be verified: refuse rather than install blind.
    let sum_name = checksum_name(name);
    let sum_url = release.assets.iter().find(|(n, _)| *n == sum_name).map(|(_, u)| u.clone()).ok_or_else(|| {
        format!("release {} publishes no checksum ({sum_name}), so the download cannot be verified", release.version)
    })?;
    for address in [&url, &sum_url] {
        if !is_release_url(address) {
            return Err(format!("{address} is not an https address on GitHub, nothing was downloaded"));
        }
    }
    let work = private_work_dir()?;
    let result = (|| {
        // The checksum first: small, and a release without a usable one is refused before the big download.
        let sums_path = work.join(&sum_name);
        fetch(&sum_url, &sums_path, MAX_SUMS)?;
        let sums = std::fs::read_to_string(&sums_path).map_err(|_| format!("{sum_name} is not a checksum file"))?;
        let archive = work.join(name);
        println!("Downloading {name}…");
        fetch(&url, &archive, MAX_ARCHIVE)?;
        verify(&archive, name, &sums)?;
        println!("Checksum verified (SHA-256).");
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

/// What to do after a successful update. Run from a NOBLE tab (the notice's "update"
/// button), the window that is still open is the old version: it has to be quit first.
fn restart_hint(inside_noble: bool) -> &'static str {
    if inside_noble {
        "Quit this NOBLE window and run `noble` again to use it (tabs come back when session restore is on)."
    } else {
        "Restart NOBLE to use the new version."
    }
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
            let inside = std::env::var_os("NOBLE_INSTANCE").is_some();
            println!("Updated noble {} → {}. {}", current(), release.version, restart_hint(inside));
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

    /// Downloads the real latest release over a temporary "installed" binary (needs network, and a latest
    /// release that publishes `.sha256` files; older ones are refused by design).
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
        Some(Release {
            version: "9.9.9".into(),
            assets: vec![
                (name.into(), "https://github.com/MertSoylu/noble/releases/download/v9.9.9/archive".into()),
                (checksum_name(name), "https://github.com/MertSoylu/noble/releases/download/v9.9.9/sums".into()),
            ],
        })
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

    /// SHA-256 of bytes, lower-case hex (through the same code path as a download).
    fn sha256_hex(bytes: &[u8]) -> String {
        let dir = std::env::temp_dir().join(format!("noble-sha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let f = dir.join(SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed).to_string());
        std::fs::write(&f, bytes).unwrap();
        let hex = sha256_file(&f).unwrap();
        let _ = std::fs::remove_file(&f);
        hex
    }

    /// A fetcher that "downloads" the archive `bytes` and the checksum file `sums`, never touching the network.
    fn serve_with(bytes: Vec<u8>, sums: String) -> impl Fn(&str, &Path, u64) -> Result<(), String> {
        move |url, to, _max| {
            let body = if url.ends_with("/sums") { sums.as_bytes() } else { &bytes[..] };
            std::fs::write(to, body).map_err(|e| e.to_string())
        }
    }

    /// Serves `bytes` with its correct checksum file (`sha256sum` format).
    fn serve(bytes: Vec<u8>) -> impl Fn(&str, &Path, u64) -> Result<(), String> {
        let sums = format!("{}  {}\n", sha256_hex(&bytes), asset_name().unwrap_or("archive"));
        serve_with(bytes, sums)
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        // Longer than one read buffer.
        let big = vec![b'a'; 1_000_000];
        assert_eq!(sha256_hex(&big), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    #[test]
    fn checksum_file_is_parsed() {
        let h = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let name = "noble-linux-x86_64.tar.gz";
        // `sha256sum` text and binary mode, CRLF, upper case, a bare hash.
        assert_eq!(parse_checksum(&format!("{h}  {name}\n"), name).as_deref(), Some(h));
        assert_eq!(parse_checksum(&format!("{h} *{name}\r\n"), name).as_deref(), Some(h));
        assert_eq!(parse_checksum(&format!("{}  {name}", h.to_uppercase()), name).as_deref(), Some(h));
        assert_eq!(parse_checksum(&format!("{h}\n"), name).as_deref(), Some(h));
        // A SHA256SUMS-style list: the line for this file.
        let other = "0".repeat(64);
        let list = format!("{other}  noble-windows-x86_64.zip\n{h}  {name}\n");
        assert_eq!(parse_checksum(&list, name).as_deref(), Some(h));
        // Another file's hash, a short hash, non-hex, an HTML page: nothing.
        assert_eq!(parse_checksum(&format!("{other}  noble-windows-x86_64.zip"), name), None);
        assert_eq!(parse_checksum(&format!("{}  {name}", &h[..63]), name), None);
        assert_eq!(parse_checksum(&format!("{}  {name}", "g".repeat(64)), name), None);
        assert_eq!(parse_checksum("<html>Not Found</html>", name), None);
        assert_eq!(parse_checksum("", name), None);
        assert_eq!(checksum_name(name), "noble-linux-x86_64.tar.gz.sha256");
    }

    #[test]
    fn checksum_mismatch_keeps_the_installed_binary() {
        let Some(release) = release_for_this_platform() else { return };
        let (dir, exe) = scratch("mismatch");
        let Some(archive) = make_archive(&dir, b"new") else { return };
        let name = asset_name().unwrap();
        let wrong = format!("{}  {name}\n", sha256_hex(b"something else"));
        let err = install_with(&release, &exe, &serve_with(archive.clone(), wrong), &tar_program()).unwrap_err();
        assert!(err.contains("checksum mismatch") && err.contains("corrupted"), "{err}");
        assert_intact(&exe);
        // A checksum file that does not name this archive (or is an error page) is refused too.
        let page = "<html>rate limited</html>".to_string();
        let err = install_with(&release, &exe, &serve_with(archive, page), &tar_program()).unwrap_err();
        assert!(err.contains("no SHA-256"), "{err}");
        assert_intact(&exe);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_without_checksum_is_refused_before_downloading() {
        let Some(mut release) = release_for_this_platform() else { return };
        release.assets.retain(|(n, _)| !n.ends_with(".sha256"));
        let (dir, exe) = scratch("nosum");
        let fetched = std::cell::Cell::new(false);
        let fetch = |_: &str, _: &Path, _: u64| -> Result<(), String> {
            fetched.set(true);
            Err("no network in tests".into())
        };
        let err = install_with(&release, &exe, &fetch, &tar_program()).unwrap_err();
        assert!(err.contains("no checksum"), "{err}");
        assert!(!fetched.get(), "downloaded an archive that cannot be verified");
        assert_intact(&exe);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Unix: the new binary takes the old one's place in a single rename, so no `.old` file is made and no
    /// staging file is left (a kill during the update never leaves the path without a binary).
    #[cfg(unix)]
    #[test]
    fn unix_update_replaces_the_binary_in_one_step() {
        let Some(release) = release_for_this_platform() else { return };
        let (dir, exe) = scratch("onestep");
        let Some(archive) = make_archive(&dir, b"new") else { return };
        install_with(&release, &exe, &serve(archive), &tar_program()).expect("update");
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert!(!old_path(&exe).exists(), "no old binary is kept on Unix");
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(names.iter().all(|n| !n.to_string_lossy().ends_with(".new")), "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verified_archive_is_installed() {
        let Some(release) = release_for_this_platform() else { return };
        let (dir, exe) = scratch("verified");
        let Some(archive) = make_archive(&dir, b"new") else { return };
        install_with(&release, &exe, &serve(archive), &tar_program()).expect("update");
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        cleanup_old_at(&exe);
        let _ = std::fs::remove_dir_all(&dir);
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
        let local = agent_with(Duration::from_secs(10), false);
        let res = download_via(&local, &format!("http://127.0.0.1:{port}/asset"), &to, MAX_ARCHIVE);
        server.join().unwrap();
        assert!(res.is_err(), "a truncated download was accepted");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_github_https_addresses_are_downloaded() {
        assert!(is_release_url("https://github.com/MertSoylu/noble/releases/download/v1/a.tar.gz"));
        assert!(is_release_url("https://objects.githubusercontent.com/github-production-release-asset/x"));
        assert!(is_release_url("https://release-assets.githubusercontent.com/x?sig=1"));
        for url in [
            "http://github.com/a",
            "https://github.com.evil.example/a",
            "https://user@github.com/a",
            "https://github.com:8443/a",
            "https://evil.example/github.com/a",
            "https:/github.com/a",
            "file:///etc/passwd",
            "",
        ] {
            assert!(!is_release_url(url), "{url}");
        }
    }

    /// A release whose asset addresses are not GitHub's is refused before anything is fetched.
    #[test]
    fn foreign_download_address_is_refused_before_fetching() {
        let Some(mut release) = release_for_this_platform() else { return };
        let name = asset_name().unwrap_or("archive");
        release.assets = vec![
            (name.into(), "https://evil.example/archive".into()),
            (checksum_name(name), "https://evil.example/sums".into()),
        ];
        let (dir, exe) = scratch("foreign");
        let fetched = std::cell::Cell::new(false);
        let fetch = |_: &str, _: &Path, _: u64| -> Result<(), String> {
            fetched.set(true);
            Ok(())
        };
        let err = install_with(&release, &exe, &fetch, &tar_program()).unwrap_err();
        assert!(err.contains("not an https address on GitHub"), "{err}");
        assert!(!fetched.get());
        assert_intact(&exe);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_entries_that_escape_or_link_are_refused() {
        // Plain files and folders pass (names as `tar -tf`, types as `tar -tvf`, GNU and bsdtar alike).
        let names = "noble\n./\n./noble\nsrc/main.rs\n";
        let kinds = "-rwxr-xr-x user/group 12 2026-01-01 00:00 noble\ndrwxr-xr-x u/g 0 2026-01-01 00:00 ./\n\
            -rwxr-xr-x u/g 1 2026-01-01 00:00 src/main.rs\n-rw-r--r-- u/g 1 2026-01-01 00:00 ./noble\n";
        assert!(check_listing(names, kinds).is_ok());
        // Each unsafe entry is refused by its own rule: absolute, `..`, a symlink, a hard link, a device.
        let cases = [
            ("/etc/cron.d/x", "-rw-r--r-- u/g 1 d x"),
            ("C:/Windows/x", "-rw-r--r-- u/g 1 d x"),
            (r"\\server\share\x", "-rw-r--r-- u/g 1 d x"),
            ("../../.bashrc", "-rw-r--r-- u/g 1 d x"),
            ("src/../../x", "-rw-r--r-- u/g 1 d x"),
            ("noble", "lrwxrwxrwx u/g 0 d noble -> /etc/passwd"),
            ("noble", "hrw-r--r-- u/g 0 d noble link to /etc/passwd"),
            ("noble", "-rw-r--r-- u/g 0 d noble link to /etc/passwd"),
            ("dev", "crw-r--r-- u/g 1, 3 d dev"),
        ];
        for (name, kind) in cases {
            assert!(check_listing(name, kind).is_err(), "{name} / {kind}");
        }
        // A listing whose two views disagree is refused too.
        assert!(check_listing("a\nb\n", "-rw-r--r-- u/g 1 d a\n").is_err());
    }

    #[test]
    fn copies_stop_at_the_size_cap() {
        let mut out = Vec::new();
        assert!(copy_capped(&mut std::io::Cursor::new(vec![7u8; 10]), &mut out, 10).is_ok());
        assert_eq!(out.len(), 10);
        let mut out = Vec::new();
        let err = copy_capped(&mut std::io::Cursor::new(vec![7u8; 11]), &mut out, 10).unwrap_err();
        assert!(err.contains("larger than 10 bytes"), "{err}");
        assert!(out.len() <= 11, "the copy did not stop at the cap");
    }

    #[test]
    fn asset_names_match_release_workflow() {
        assert_eq!(asset_name_for("windows", "x86_64"), Some("noble-windows-x86_64.zip"));
        assert_eq!(asset_name_for("linux", "x86_64"), Some("noble-linux-x86_64.tar.gz"));
        assert_eq!(asset_name_for("linux", "aarch64"), Some("noble-linux-aarch64.tar.gz"));
        assert_eq!(asset_name_for("macos", "aarch64"), Some("noble-macos-aarch64.tar.gz"));
        assert_eq!(asset_name_for("macos", "x86_64"), Some("noble-macos-x86_64.tar.gz"));
        assert_eq!(asset_name_for("freebsd", "x86_64"), None);
        assert_eq!(asset_name(), asset_name_for(std::env::consts::OS, std::env::consts::ARCH));
        // Every archive release.yml builds is the one `noble update` asks for on that target.
        let yml =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/.github/workflows/release.yml")).unwrap();
        let field = |line: &str, key: &str| line.trim().strip_prefix(key).map(|v| v.trim().to_string());
        let targets: Vec<String> =
            yml.lines().filter_map(|l| field(l, "target:")).filter(|t| !t.contains('$')).collect();
        let archives: Vec<String> = yml.lines().filter_map(|l| field(l, "archive:")).collect();
        assert_eq!(targets.len(), 6, "{targets:?}");
        assert_eq!(targets.len(), archives.len());
        for (target, archive) in targets.iter().zip(&archives) {
            let arch = target.split('-').next().unwrap();
            let os = if target.contains("windows") {
                "windows"
            } else if target.contains("apple-darwin") {
                "macos"
            } else {
                "linux"
            };
            assert_eq!(asset_name_for(os, arch), Some(archive.as_str()), "{target}");
        }
        // Each archive is published with the checksum file `noble update` verifies it against.
        let sum = checksum_name("${{ matrix.archive }}");
        assert!(yml.matches(&sum).count() >= 2, "release.yml must write and upload {sum}");
    }

    #[test]
    fn old_binary_sits_next_to_the_exe() {
        let p = old_path(Path::new("/usr/bin/noble"));
        assert_eq!(p, Path::new("/usr/bin/noble.old"));
    }

    #[test]
    fn work_dir_is_fresh_and_private() {
        let a = private_work_dir().unwrap();
        let b = private_work_dir().unwrap();
        assert_ne!(a, b);
        assert!(std::fs::read_dir(&a).unwrap().next().is_none(), "a new directory is empty");
        // Unix: the shared temp dir must not let other users look in or plant files. Windows: %TEMP% is per user.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let _ = std::fs::remove_dir(&a);
        let _ = std::fs::remove_dir(&b);
    }
}
