//! Listening TCP ports per project folder. A port belongs to the project its listening process runs in
//! (the process's current directory). Home's project card shows them.
//!
//! Linux reads `/proc` (sockets and their processes), macOS asks `lsof`, Windows asks the IP Helper
//! owner table (`GetExtendedTcpTable`). The process directories come from sysinfo on macOS and Windows,
//! from the `/proc/<pid>/cwd` links on Linux. macOS filters first: sysinfo finds the processes inside
//! project folders, and `lsof` runs only for their pids, so an idle Home (no such process) runs no `lsof`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::event::{AppEvent, Tx};
use crate::util;

/// Listening ports of the given project folders: `(folder, ports)` for every folder that has any, the
/// ports sorted and unique, the folders in the caller's order and spelling. `skip_pid` (NOBLE itself) is
/// never counted. Processes that cannot be inspected are skipped silently; this never panics.
pub fn scan(roots: &[PathBuf], skip_pid: Option<u32>) -> Vec<(PathBuf, Vec<u16>)> {
    if roots.is_empty() {
        return Vec::new();
    }
    let index = Roots::new(roots);
    let mut found: Vec<Vec<u16>> = vec![Vec::new(); index.groups.len()];
    for (group, port) in listening(&index, skip_pid) {
        found[group].push(port);
    }
    for ports in &mut found {
        ports.sort_unstable();
        ports.dedup();
    }
    roots
        .iter()
        .zip(&index.group_of)
        .filter(|(_, group)| !found[**group].is_empty())
        .map(|(root, group)| (root.clone(), found[*group].clone()))
        .collect()
}

/// Background thread: each request (project folders) is answered with a scan of them.
pub fn spawn(tx: Tx, requests: std::sync::mpsc::Receiver<Vec<PathBuf>>) {
    let _ = std::thread::Builder::new().name("ports".into()).spawn(move || {
        for roots in requests {
            let list = scan(&roots, Some(std::process::id()));
            if tx.send(AppEvent::Ports(list)).is_err() {
                break;
            }
        }
    });
}

/// The project folders with their real locations, so that a process's folder (always a real path) can be
/// matched: symlinks and macOS's `/var` → `/private/var` are resolved once per scan.
struct Roots {
    /// One entry per distinct folder.
    groups: Vec<Group>,
    /// The group of each root, in the caller's order.
    group_of: Vec<usize>,
}

struct Group {
    key: String,
    /// `key` with a trailing `/`: the prefix every folder inside it starts with.
    prefix: String,
}

impl Roots {
    fn new(roots: &[PathBuf]) -> Roots {
        let mut groups: Vec<Group> = Vec::new();
        let mut group_of = Vec::with_capacity(roots.len());
        for root in roots {
            // A folder that does not exist (any more) is compared as it is written.
            let real = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
            let key = key(&real);
            let group = match groups.iter().position(|g| g.key == key) {
                Some(g) => g,
                None => {
                    let prefix = if key.ends_with('/') { key.clone() } else { format!("{key}/") };
                    groups.push(Group { key, prefix });
                    groups.len() - 1
                }
            };
            group_of.push(group);
        }
        Roots { groups, group_of }
    }

    /// The deepest project folder that contains `dir` (or is `dir`).
    fn owner(&self, dir: &Path) -> Option<usize> {
        let dir = key(dir);
        self.groups
            .iter()
            .enumerate()
            .filter(|(_, g)| dir == g.key || dir.starts_with(&g.prefix))
            .max_by_key(|(_, g)| g.key.len())
            .map(|(i, _)| i)
    }
}

/// A comparable form of a folder path (`util::path_key`: `/` separators, no trailing separator, lowercase
/// where the file system ignores case). A Windows canonical path's `\\?\` prefix is removed first.
fn key(path: &Path) -> String {
    let text = path.to_string_lossy();
    let plain = if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(local) = text.strip_prefix(r"\\?\") {
        local.to_string()
    } else {
        text.into_owned()
    };
    util::path_key(Path::new(&plain))
}

/// Listening sockets of processes inside a project folder, as `(group, port)`.
#[cfg(target_os = "linux")]
fn listening(roots: &Roots, skip_pid: Option<u32>) -> Vec<(usize, u16)> {
    let mut port_of_inode = HashMap::new();
    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = std::fs::read_to_string(table) {
            port_of_inode.extend(parse_proc_net_tcp(&text));
        }
    }
    if port_of_inode.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return out };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        if Some(pid) == skip_pid {
            continue;
        }
        // The cheap test first: only a process inside a project folder has its sockets looked at.
        let Ok(cwd) = std::fs::read_link(format!("/proc/{pid}/cwd")) else { continue };
        let Some(group) = roots.owner(&cwd) else { continue };
        let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else { continue };
        for fd in fds.flatten() {
            let Ok(target) = std::fs::read_link(fd.path()) else { continue };
            let Some(inode) = socket_inode(&target) else { continue };
            if let Some(port) = port_of_inode.get(&inode) {
                out.push((group, *port));
            }
        }
    }
    out
}

/// macOS: sysinfo finds the processes whose current directory is in a project folder. `lsof` runs only when
/// there are such processes, and only for their pids (`-a -p` ANDs the selections), so it never walks the
/// file descriptors of every process on the machine.
#[cfg(target_os = "macos")]
fn listening(roots: &Roots, skip_pid: Option<u32>) -> Vec<(usize, u16)> {
    let folders = folders_by_pid(roots, skip_pid, None);
    if folders.is_empty() {
        return Vec::new();
    }
    let mut pids: Vec<u32> = folders.keys().copied().collect();
    pids.sort_unstable();
    let list = pids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    attribute(&folders, &lsof_listeners(&list))
}

/// Windows: the IP Helper owner tables list the listening sockets with their process; sysinfo is asked
/// only for the folders of those processes.
#[cfg(windows)]
fn listening(roots: &Roots, skip_pid: Option<u32>) -> Vec<(usize, u16)> {
    let listeners = windows_listeners();
    let mut pids: Vec<u32> = listeners.iter().map(|(pid, _)| *pid).collect();
    pids.sort_unstable();
    pids.dedup();
    if pids.is_empty() {
        return Vec::new();
    }
    let folders = folders_by_pid(roots, skip_pid, Some(&pids));
    attribute(&folders, &listeners)
}

/// Other systems (the BSDs): no port list is read, so the project cards show no ports there.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn listening(roots: &Roots, skip_pid: Option<u32>) -> Vec<(usize, u16)> {
    let _ = (roots, skip_pid);
    Vec::new()
}

/// The folder of each process, by pid: of every process when `only` is `None`, else of the listed pids.
/// sysinfo reads each process's current directory; a process it cannot inspect has no folder. NOBLE's own
/// process (`skip_pid`) and pid 0 are left out.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
fn folders_by_pid(roots: &Roots, skip_pid: Option<u32>, only: Option<&[u32]>) -> HashMap<u32, usize> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let refresh = ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always);
    let mut sys = System::new();
    match only {
        Some(pids) => {
            let pids: Vec<Pid> = pids.iter().map(|pid| Pid::from_u32(*pid)).collect();
            sys.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), true, refresh);
        }
        None => {
            sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
        }
    }
    let mut folders = HashMap::new();
    for (pid, process) in sys.processes() {
        let pid = pid.as_u32();
        if pid == 0 || Some(pid) == skip_pid || !only.is_none_or(|list| list.contains(&pid)) {
            continue;
        }
        if let Some(group) = process.cwd().and_then(|cwd| roots.owner(cwd)) {
            folders.insert(pid, group);
        }
    }
    folders
}

/// The listeners whose process has a folder, as `(folder, port)`.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
fn attribute(folders: &HashMap<u32, usize>, listeners: &[(u32, u16)]) -> Vec<(usize, u16)> {
    listeners.iter().filter_map(|(pid, port)| folders.get(pid).map(|group| (*group, *port))).collect()
}

/// The inode of a socket file descriptor (`socket:[1234]`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn socket_inode(link: &Path) -> Option<u64> {
    link.to_str()?.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
}

/// Listening rows of a `/proc/net/tcp` or `/proc/net/tcp6` table: inode → local port. Only state `0A`
/// (LISTEN) counts; the header and malformed lines are skipped.
pub fn parse_proc_net_tcp(text: &str) -> HashMap<u64, u16> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // `sl local_address rem_address st ... inode`: state is field 3, local address 1, inode 9.
        if fields.len() < 10 || fields[3] != "0A" {
            continue;
        }
        let Some(port) = fields[1].rsplit_once(':').and_then(|(_, p)| u16::from_str_radix(p, 16).ok()) else {
            continue;
        };
        let Ok(inode) = fields[9].parse::<u64>() else { continue };
        out.insert(inode, port);
    }
    out
}

/// `lsof -nP -iTCP -sTCP:LISTEN -Fpn` output as `(pid, port)`. `p<pid>` starts a process, `n<address>`
/// is one of its listening addresses: `*:5173`, `127.0.0.1:3000` or `[::1]:3000`.
pub fn parse_lsof(text: &str) -> Vec<(u32, u16)> {
    let mut out = Vec::new();
    let mut owner: Option<u32> = None;
    for line in text.lines() {
        if let Some(pid) = line.strip_prefix('p') {
            owner = pid.trim().parse().ok();
        } else if let (Some(address), Some(pid)) = (line.strip_prefix('n'), owner) {
            let address = address.split_whitespace().next().unwrap_or("");
            if let Some(port) = address.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok()) {
                out.push((pid, port));
            }
        }
    }
    out
}

/// macOS: the listening TCP sockets of the listed pids (comma-separated) as `(pid, port)`, from `lsof`
/// (`/usr/sbin/lsof` ships with macOS; the PATH is searched only without it). A failure or a slow answer
/// gives none.
#[cfg(target_os = "macos")]
fn lsof_listeners(pids: &str) -> Vec<(u32, u16)> {
    let fixed = Path::new("/usr/sbin/lsof");
    let program = if fixed.is_file() {
        fixed.to_path_buf()
    } else {
        match util::which("lsof") {
            Some(program) => program,
            None => return Vec::new(),
        }
    };
    let mut cmd = util::command_for(&program);
    cmd.args(["-nP", "-a", "-p", pids, "-iTCP", "-sTCP:LISTEN", "-Fpn"]);
    crate::ai::run_command(cmd, std::time::Duration::from_secs(3)).map(|out| parse_lsof(&out)).unwrap_or_default()
}

/// Windows: the listening TCP sockets of both address families as `(pid, port)`.
#[cfg(windows)]
fn windows_listeners() -> Vec<(u32, u16)> {
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
    let mut out = owner_table(u32::from(AF_INET), TCP4_ROW);
    out.extend(owner_table(u32::from(AF_INET6), TCP6_ROW));
    out
}

/// One address family's owner table (`GetExtendedTcpTable`, listeners only). The size is asked first, then
/// the table is fetched into a buffer of that size; the table may grow in between, so the size is retried.
#[cfg(windows)]
fn owner_table(family: u32, row: OwnerRow) -> Vec<(u32, u16)> {
    use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, TCP_TABLE_OWNER_PID_LISTENER};
    let mut size = 0u32;
    let mut table: Vec<u8> = Vec::new();
    for _ in 0..4 {
        let ptr = if table.is_empty() { std::ptr::null_mut() } else { table.as_mut_ptr().cast() };
        // SAFETY: a null pointer with a size of 0 asks for the size only. Otherwise `ptr` points to `table`,
        // which holds `size` bytes, and the call writes no more than that; `size` is a valid out pointer.
        let rc = unsafe { GetExtendedTcpTable(ptr, &mut size, 0, family, TCP_TABLE_OWNER_PID_LISTENER, 0) };
        match rc {
            0 => return table.get(..size as usize).map(|t| parse_owner_table(t, row)).unwrap_or_default(),
            ERROR_INSUFFICIENT_BUFFER => table = vec![0u8; size as usize],
            _ => return Vec::new(),
        }
    }
    Vec::new()
}

/// Byte layout of one row of an IP Helper owner table (`MIB_TCPROW_OWNER_PID`, `MIB_TCP6ROW_OWNER_PID`).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy)]
struct OwnerRow {
    len: usize,
    local_port: usize,
    owning_pid: usize,
}

/// `MIB_TCPROW_OWNER_PID`: state, local address, local port, remote address, remote port, owning pid.
#[cfg_attr(not(windows), allow(dead_code))]
const TCP4_ROW: OwnerRow = OwnerRow { len: 24, local_port: 8, owning_pid: 20 };
/// `MIB_TCP6ROW_OWNER_PID`: local address (16), scope id, local port, remote address (16), scope id, remote
/// port, state, owning pid.
#[cfg_attr(not(windows), allow(dead_code))]
const TCP6_ROW: OwnerRow = OwnerRow { len: 56, local_port: 20, owning_pid: 52 };

#[cfg(windows)]
const _: () = {
    use windows_sys::Win32::NetworkManagement::IpHelper::{MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID};
    // The offsets above must match the structs the API fills in.
    assert!(std::mem::size_of::<MIB_TCPROW_OWNER_PID>() == TCP4_ROW.len);
    assert!(std::mem::offset_of!(MIB_TCPROW_OWNER_PID, dwLocalPort) == TCP4_ROW.local_port);
    assert!(std::mem::offset_of!(MIB_TCPROW_OWNER_PID, dwOwningPid) == TCP4_ROW.owning_pid);
    assert!(std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>() == TCP6_ROW.len);
    assert!(std::mem::offset_of!(MIB_TCP6ROW_OWNER_PID, dwLocalPort) == TCP6_ROW.local_port);
    assert!(std::mem::offset_of!(MIB_TCP6ROW_OWNER_PID, dwOwningPid) == TCP6_ROW.owning_pid);
};

/// The `(pid, port)` rows of an owner table: a `DWORD` entry count, then the rows. The port is in network
/// byte order in the low 16 bits of its `DWORD`, so `from_be` reads it the way `ntohs` does.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_owner_table(table: &[u8], row: OwnerRow) -> Vec<(u32, u16)> {
    let Some(count) = read_u32(table, 0) else { return Vec::new() };
    let Some(rows) = table.get(4..) else { return Vec::new() };
    rows.chunks_exact(row.len)
        .take(count as usize)
        .filter_map(|r| {
            let pid = read_u32(r, row.owning_pid)?;
            let port = u16::from_be(read_u32(r, row.local_port)? as u16);
            Some((pid, port))
        })
        .collect()
}

/// A native-endian `u32` at a byte offset.
#[cfg_attr(not(windows), allow(dead_code))]
fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_net_tcp_keeps_only_listening_rows() {
        let text = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:0BB8 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 41231 1 ffff8f1e 100 0 0 10 0
   1: 0100007F:1F90 0100007F:D2F4 01 00000000:00000000 00:00000000 00000000  1000        0 55555 1 ffff8f1e 100 0 0 10 0
   2: 00000000:1F91 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 42000 1 ffff8f1e 100 0 0 10 0
   3: 0000000000000000FFFF00000100007F:0016 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 77 1 ffff8f1e 100 0 0 10 0
   4: garbage line
";
        let map = parse_proc_net_tcp(text);
        assert_eq!(map.len(), 3);
        assert_eq!(map.get(&41231), Some(&3000));
        assert_eq!(map.get(&42000), Some(&8081));
        assert_eq!(map.get(&77), Some(&22));
        assert!(!map.contains_key(&55555), "established rows are not listeners");
        assert!(parse_proc_net_tcp("").is_empty());
    }

    #[test]
    fn lsof_addresses_give_ports() {
        let text = "p123\nn*:5173\nn127.0.0.1:3000\nf5\np456\nn[::1]:8080\nn[::1]:8080\nn*:http\nn*:9000 (LISTEN)\n";
        assert_eq!(parse_lsof(text), vec![(123, 5173), (123, 3000), (456, 8080), (456, 8080), (456, 9000)],);
        // An address before any process line has no owner.
        assert!(parse_lsof("n*:5173\n").is_empty());
        assert!(parse_lsof("").is_empty());
    }

    #[test]
    fn listeners_take_the_folder_of_their_process() {
        let folders = HashMap::from([(10u32, 0usize), (11, 1)]);
        let listeners = [(10, 3000), (12, 4000), (11, 5173), (10, 3000)];
        assert_eq!(attribute(&folders, &listeners), vec![(0, 3000), (1, 5173), (0, 3000)]);
        assert!(attribute(&folders, &[(12, 4000)]).is_empty());
    }

    #[test]
    fn socket_links_name_their_inode() {
        assert_eq!(socket_inode(Path::new("socket:[41231]")), Some(41231));
        assert_eq!(socket_inode(Path::new("/dev/null")), None);
        assert_eq!(socket_inode(Path::new("socket:[x]")), None);
    }

    #[test]
    fn owner_table_rows_give_pid_and_port() {
        fn tcp4_row(pid: u32, port: u16) -> Vec<u8> {
            let mut row = vec![0u8; 24];
            row[8..12].copy_from_slice(&u32::from(port.to_be()).to_ne_bytes());
            row[20..24].copy_from_slice(&pid.to_ne_bytes());
            row
        }
        let mut table = 2u32.to_ne_bytes().to_vec();
        table.extend(tcp4_row(4321, 3000));
        table.extend(tcp4_row(99, 443));
        assert_eq!(parse_owner_table(&table, TCP4_ROW), vec![(4321, 3000), (99, 443)]);
        // A truncated table yields the complete rows only; a broken header yields nothing.
        table.truncate(4 + 24 + 10);
        assert_eq!(parse_owner_table(&table, TCP4_ROW), vec![(4321, 3000)]);
        assert!(parse_owner_table(&[1, 2], TCP4_ROW).is_empty());
    }

    #[test]
    fn a_listener_is_attributed_to_its_folder() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let here = std::env::current_dir().unwrap();
        let found = scan(std::slice::from_ref(&here), None);
        let ports = found.iter().find(|(p, _)| *p == here).map(|(_, ports)| ports.clone()).unwrap_or_default();
        assert!(ports.contains(&port), "port {port} not attributed to {}: {found:?}", here.display());
        // Our own process is skipped when asked to be.
        let own = scan(std::slice::from_ref(&here), Some(std::process::id()));
        assert!(!own.iter().any(|(_, ports)| ports.contains(&port)));
        // A folder elsewhere gets nothing from this process.
        let elsewhere = std::env::temp_dir().join(format!("noble-ports-test-{}", std::process::id()));
        std::fs::create_dir_all(&elsewhere).unwrap();
        let far = scan(std::slice::from_ref(&elsewhere), None);
        assert!(!far.iter().any(|(_, ports)| ports.contains(&port)));
        let _ = std::fs::remove_dir(&elsewhere);
        drop(listener);
    }

    #[test]
    fn the_deepest_folder_gets_the_port() {
        let here = std::env::current_dir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // The crate folder and its parent both contain this process: only the deeper one is credited.
        let parent = here.parent().map(Path::to_path_buf).unwrap_or_else(|| here.clone());
        let found = scan(&[parent.clone(), here.clone()], None);
        assert!(!found.iter().any(|(p, v)| *p == parent && v.contains(&port)), "{found:?}");
        assert!(found.iter().any(|(p, v)| *p == here && v.contains(&port)), "{found:?}");
        drop(listener);
    }

    /// Home scans the ports every 10 s, so a scan must stay cheap on every platform. Two cases: no process
    /// inside a project folder (the usual one; macOS runs no `lsof` then) and a listener inside one (macOS
    /// runs `lsof` for it). Wall time is measured because it also covers the `lsof` child's work, so it is an
    /// upper bound of the CPU cost; a busy CI runner only makes it larger. The result line is written past
    /// the test harness's output capture on purpose, so every CI log shows the numbers.
    #[test]
    fn a_scan_stays_cheap() {
        use std::io::Write;
        use std::time::{Duration, Instant};
        const RUNS: u32 = 20;
        let tag = std::process::id();
        let elsewhere: Vec<PathBuf> =
            (0..60).map(|i| std::env::temp_dir().join(format!("noble-ports-cost-{tag}-{i}"))).collect();
        for dir in &elsewhere {
            std::fs::create_dir_all(dir).unwrap();
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        // The fastest of `RUNS` scans: other tests running in parallel only ever add time to a scan, so the minimum
        // is the scan's own cost and stays put on a busy machine.
        let per_scan = |roots: &[PathBuf]| {
            let _ = scan(roots, None);
            (0..RUNS)
                .map(|_| {
                    let start = Instant::now();
                    let _ = scan(roots, None);
                    start.elapsed()
                })
                .min()
                .unwrap_or_default()
        };
        let idle = per_scan(&elsewhere);
        let mut with_listener = elsewhere.clone();
        with_listener.push(std::env::current_dir().unwrap());
        let busy = per_scan(&with_listener);
        // At one scan per 10 s (`app::live::PORTS_EVERY`), a minute holds 6 scans.
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let line = format!(
            "ports::scan on {} ({} folders): no project process {:.2} ms/scan (≤ {:.0} ms CPU/min on Home), \
             a listener in a project {:.2} ms/scan (≤ {:.0} ms CPU/min on Home)\n",
            std::env::consts::OS,
            with_listener.len(),
            ms(idle),
            ms(idle) * 6.0,
            ms(busy),
            ms(busy) * 6.0,
        );
        let _ = std::io::stdout().write_all(line.as_bytes());
        for dir in &elsewhere {
            let _ = std::fs::remove_dir(dir);
        }
        drop(listener);
        // Generous limits: they catch a scan that turned expensive, not the noise of a shared runner. macOS runs `lsof`
        // for the listener, which is slow there, so its limit is wider.
        let busy_limit =
            if cfg!(target_os = "macos") { Duration::from_millis(1000) } else { Duration::from_millis(250) };
        assert!(idle < Duration::from_millis(50), "{line}");
        assert!(busy < busy_limit, "{line}");
    }
}
