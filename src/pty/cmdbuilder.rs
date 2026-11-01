#[cfg(unix)]
use anyhow::Context;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(unix)]
use std::path::Component;
use std::path::Path;

/// Used to deal with Windows having case-insensitive environment variables.
#[derive(Clone, Debug, PartialEq, PartialOrd)]
struct EnvEntry {
    /// Whether or not this environment variable came from the base environment,
    /// as opposed to having been explicitly set by the caller.
    is_from_base_env: bool,

    /// For case-insensitive platforms, the environment variable key in its preferred casing.
    preferred_key: OsString,

    /// The environment variable value.
    value: OsString,
}

impl EnvEntry {
    fn map_key(k: OsString) -> OsString {
        if cfg!(windows) {
            // Best-effort lowercase transformation of an os string
            match k.to_str() {
                Some(s) => s.to_lowercase().into(),
                None => k,
            }
        } else {
            k
        }
    }
}

#[cfg(unix)]
fn get_shell() -> String {
    use nix::unistd::{AccessFlags, access};
    use std::ffi::CStr;
    use std::str;

    let ent = unsafe { libc::getpwuid(libc::getuid()) };
    if !ent.is_null() {
        let shell = unsafe { CStr::from_ptr((*ent).pw_shell) };
        match shell.to_str().map(str::to_owned) {
            Err(err) => {
                log::warn!(
                    "passwd database shell could not be \
                     represented as utf-8: {err:#}, \
                     falling back to /bin/sh"
                );
            }
            Ok(shell) => {
                if let Err(err) = access(Path::new(&shell), AccessFlags::X_OK) {
                    log::warn!(
                        "passwd database shell={shell:?} which is \
                         not executable ({err:#}), falling back to /bin/sh"
                    );
                } else {
                    return shell;
                }
            }
        }
    }
    "/bin/sh".into()
}

fn get_base_env() -> BTreeMap<OsString, EnvEntry> {
    let mut env: BTreeMap<OsString, EnvEntry> = std::env::vars_os()
        .map(|(key, value)| {
            (EnvEntry::map_key(key.clone()), EnvEntry { is_from_base_env: true, preferred_key: key, value })
        })
        .collect();

    #[cfg(unix)]
    {
        let key = EnvEntry::map_key("SHELL".into());
        // Only set the value of SHELL if it isn't already set
        if !env.contains_key(&key) {
            env.insert(
                EnvEntry::map_key("SHELL".into()),
                EnvEntry { is_from_base_env: true, preferred_key: "SHELL".into(), value: get_shell().into() },
            );
        }
    }

    #[cfg(windows)]
    {
        // The system's variables, then the user's (the user's `Path` is appended to the system's). Unix has no
        // registry: its base environment is the process's own, as the code above reads it.
        for (name, value) in registry::string_values(
            registry::HKEY_LOCAL_MACHINE,
            "System\\CurrentControlSet\\Control\\Session Manager\\Environment",
        ) {
            if name.eq_ignore_ascii_case("username") {
                continue;
            }
            log::trace!("adding SYS env: {:?} {:?}", name, value);
            env.insert(
                EnvEntry::map_key(name.clone().into()),
                EnvEntry { is_from_base_env: true, preferred_key: name.into(), value },
            );
        }

        for (name, value) in registry::string_values(registry::HKEY_CURRENT_USER, "Environment") {
            // Merge the system and user paths together
            let value = if name.eq_ignore_ascii_case("path") {
                match env.get(&EnvEntry::map_key(name.clone().into())) {
                    Some(entry) => {
                        let mut result = OsString::new();
                        result.push(&entry.value);
                        result.push(";");
                        result.push(&value);
                        result
                    }
                    None => value,
                }
            } else {
                value
            };

            log::trace!("adding USER env: {:?} {:?}", name, value);
            env.insert(
                EnvEntry::map_key(name.clone().into()),
                EnvEntry { is_from_base_env: true, preferred_key: name.into(), value },
            );
        }
    }

    env
}

/// The registry reads behind the Windows base environment. They were `winreg` 0.10 calls (which pulled in winapi);
/// they are the windows-sys calls of the same Win32 functions, with the same results for this use: a value that is
/// not a string type, or whose name is not valid UTF-16, is skipped, and a key that does not open gives no values.
#[cfg(windows)]
mod registry {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::ptr;
    use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows_sys::Win32::System::Registry::{
        HKEY, KEY_READ, REG_EXPAND_SZ, REG_MULTI_SZ, REG_SZ, RegCloseKey, RegEnumValueW, RegOpenKeyExW,
    };

    pub use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    /// The string values of `subkey` under `root`, as `(name, value)` in the order the registry lists them.
    /// `REG_EXPAND_SZ` values come back expanded, and trailing NUL characters are dropped.
    pub fn string_values(root: HKEY, subkey: &str) -> Vec<(String, OsString)> {
        let path: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
        let mut key: HKEY = ptr::null_mut();
        if unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ, &mut key) } != ERROR_SUCCESS {
            return Vec::new();
        }
        let values = enum_values(key);
        unsafe { RegCloseKey(key) };
        values
    }

    /// Every value of an open key, read by index like `winreg`'s `enum_values`: a value that errors is skipped.
    fn enum_values(key: HKEY) -> Vec<(String, OsString)> {
        // A value name is at most 16,383 characters; with its terminating NUL that fits in this buffer.
        let mut name = vec![0u16; 16_384];
        let mut data = vec![0u8; 2048];
        let mut values = Vec::new();
        let mut index = 0u32;
        loop {
            let mut name_len = name.len() as u32;
            let mut kind = 0u32;
            let mut data_len = data.len() as u32;
            let status = unsafe {
                RegEnumValueW(
                    key,
                    index,
                    name.as_mut_ptr(),
                    &mut name_len,
                    ptr::null(),
                    &mut kind,
                    data.as_mut_ptr(),
                    &mut data_len,
                )
            };
            match status {
                ERROR_NO_MORE_ITEMS => break,
                ERROR_MORE_DATA => {
                    // `data_len` is the size the value needs; grow past the old size too, so this always ends.
                    let needed = (data_len as usize).max(data.len() * 2);
                    data.resize(needed, 0);
                    continue;
                }
                ERROR_SUCCESS => {
                    let name_chars = &name[..name_len as usize];
                    let bytes = &data[..data_len as usize];
                    if let Some(value) = string_value(name_chars, kind, bytes) {
                        values.push(value);
                    }
                }
                _ => {}
            }
            index += 1;
        }
        values
    }

    /// One value as NOBLE takes it: `None` for a name that is not valid UTF-16 or a type that is not a string.
    fn string_value(name: &[u16], kind: u32, bytes: &[u8]) -> Option<(String, OsString)> {
        let name = String::from_utf16(name).ok()?;
        // The registry stores UTF-16LE. The bytes are decoded one pair at a time: a Vec<u8> need not be 2-byte aligned.
        let words: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        let value = match kind {
            REG_SZ | REG_MULTI_SZ => OsString::from_wide(trim_nuls(&words)),
            REG_EXPAND_SZ => {
                // ExpandEnvironmentStringsW reads up to the first NUL, so the source is made NUL terminated first.
                let mut src = words;
                src.push(0);
                OsString::from_wide(trim_nuls(&expand(&src)))
            }
            _ => return None,
        };
        Some((name, value))
    }

    /// Expands the `%NAME%` references of a NUL terminated string with this process's environment.
    fn expand(src: &[u16]) -> Vec<u16> {
        let size = unsafe { ExpandEnvironmentStringsW(src.as_ptr(), ptr::null_mut(), 0) };
        let mut buf = vec![0u16; size as usize + 1];
        unsafe { ExpandEnvironmentStringsW(src.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
        buf
    }

    fn trim_nuls(mut words: &[u16]) -> &[u16] {
        while let Some((0, rest)) = words.split_last() {
            words = rest;
        }
        words
    }
}

/// `CommandBuilder` is used to prepare a command to be spawned into a pty.
/// The interface is intentionally similar to that of `std::process::Command`.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandBuilder {
    args: Vec<OsString>,
    envs: BTreeMap<OsString, EnvEntry>,
    cwd: Option<OsString>,
    #[cfg(unix)]
    pub(crate) umask: Option<libc::mode_t>,
    controlling_tty: bool,
}

impl CommandBuilder {
    /// Create a new builder instance with argv\[0\] set to the specified
    /// program.
    pub fn new<S: AsRef<OsStr>>(program: S) -> Self {
        Self {
            args: vec![program.as_ref().to_owned()],
            envs: get_base_env(),
            cwd: None,
            #[cfg(unix)]
            umask: None,
            controlling_tty: true,
        }
    }

    /// Create a new builder instance from a pre-built argument vector
    pub fn from_argv(args: Vec<OsString>) -> Self {
        Self {
            args,
            envs: get_base_env(),
            cwd: None,
            #[cfg(unix)]
            umask: None,
            controlling_tty: true,
        }
    }

    /// Set whether we should set the pty as the controlling terminal.
    /// The default is true, which is usually what you want, but you
    /// may need to set this to false if you are crossing container
    /// boundaries (eg: flatpak) to workaround issues like:
    /// <https://github.com/flatpak/flatpak/issues/3697>
    /// <https://github.com/flatpak/flatpak/issues/3285>
    pub fn set_controlling_tty(&mut self, controlling_tty: bool) {
        self.controlling_tty = controlling_tty;
    }

    pub fn get_controlling_tty(&self) -> bool {
        self.controlling_tty
    }

    /// Create a new builder instance that will run some idea of a default
    /// program.  Such a builder will panic if `arg` is called on it.
    pub fn new_default_prog() -> Self {
        Self {
            args: vec![],
            envs: get_base_env(),
            cwd: None,
            #[cfg(unix)]
            umask: None,
            controlling_tty: true,
        }
    }

    /// Returns true if this builder was created via `new_default_prog`
    pub fn is_default_prog(&self) -> bool {
        self.args.is_empty()
    }

    /// Append an argument to the current command line.
    /// Will panic if called on a builder created via `new_default_prog`.
    pub fn arg<S: AsRef<OsStr>>(&mut self, arg: S) {
        if self.is_default_prog() {
            panic!("attempted to add args to a default_prog builder");
        }
        self.args.push(arg.as_ref().to_owned());
    }

    /// If a builder is_default_prog, then this function can be used to
    /// set the actual prog that should be used.
    /// This is intended to facilitate plumbing through the handling
    /// of the underlying default prog when merging together supplemental
    /// env and cwd information.
    /// You will not typically use this method in your own code.
    pub fn replace_default_prog(&mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) {
        if !self.is_default_prog() {
            panic!("attempted to replace_default_prog on a non-default_prog builder");
        }

        for arg in args {
            self.args.push(arg.as_ref().to_owned());
        }
    }

    /// Append a sequence of arguments to the current command line
    pub fn args<I, S>(&mut self, args: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self.arg(arg);
        }
    }

    pub fn get_argv(&self) -> &Vec<OsString> {
        &self.args
    }

    pub fn get_argv_mut(&mut self) -> &mut Vec<OsString> {
        &mut self.args
    }

    /// Override the value of an environmental variable
    pub fn env<K, V>(&mut self, key: K, value: V)
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let key: OsString = key.as_ref().into();
        let value: OsString = value.as_ref().into();
        self.envs
            .insert(EnvEntry::map_key(key.clone()), EnvEntry { is_from_base_env: false, preferred_key: key, value });
    }

    pub fn env_remove<K>(&mut self, key: K)
    where
        K: AsRef<OsStr>,
    {
        let key = key.as_ref().into();
        self.envs.remove(&EnvEntry::map_key(key));
    }

    pub fn env_clear(&mut self) {
        self.envs.clear();
    }

    pub fn get_env<K>(&self, key: K) -> Option<&OsStr>
    where
        K: AsRef<OsStr>,
    {
        let key = key.as_ref().into();
        self.envs
            .get(&EnvEntry::map_key(key))
            .map(|EnvEntry { is_from_base_env: _, preferred_key: _, value }| value.as_os_str())
    }

    pub fn cwd<D>(&mut self, dir: D)
    where
        D: AsRef<OsStr>,
    {
        self.cwd = Some(dir.as_ref().to_owned());
    }

    pub fn clear_cwd(&mut self) {
        self.cwd.take();
    }

    pub fn get_cwd(&self) -> Option<&OsString> {
        self.cwd.as_ref()
    }

    /// Iterate over the configured environment. Only includes environment
    /// variables set by the caller via `env`, not variables set in the base
    /// environment.
    pub fn iter_extra_env_as_str(&self) -> impl Iterator<Item = (&str, &str)> {
        self.envs.values().filter_map(|EnvEntry { is_from_base_env, preferred_key, value }| {
            if *is_from_base_env {
                None
            } else {
                let key = preferred_key.to_str()?;
                let value = value.to_str()?;
                Some((key, value))
            }
        })
    }

    pub fn iter_full_env_as_str(&self) -> impl Iterator<Item = (&str, &str)> {
        self.envs.values().filter_map(|EnvEntry { preferred_key, value, .. }| {
            let key = preferred_key.to_str()?;
            let value = value.to_str()?;
            Some((key, value))
        })
    }

    /// Return the configured command and arguments as a single string,
    /// quoted per the unix shell conventions.
    pub fn as_unix_command_line(&self) -> anyhow::Result<String> {
        let mut strs = vec![];
        for arg in &self.args {
            let s = arg.to_str().ok_or_else(|| anyhow::anyhow!("argument cannot be represented as utf8"))?;
            strs.push(s);
        }
        Ok(shell_words::join(strs))
    }
}

#[cfg(unix)]
impl CommandBuilder {
    pub fn umask(&mut self, mask: Option<libc::mode_t>) {
        self.umask = mask;
    }

    fn resolve_path(&self) -> Option<&OsStr> {
        self.get_env("PATH")
    }

    fn search_path(&self, exe: &OsStr, cwd: &OsStr) -> anyhow::Result<OsString> {
        use nix::unistd::{AccessFlags, access};

        let exe_path: &Path = exe.as_ref();
        if exe_path.is_relative() {
            let cwd: &Path = cwd.as_ref();
            let mut errors = vec![];

            // If the requested executable is explicitly relative to cwd,
            // then check only there.
            if is_cwd_relative_path(exe_path) {
                let abs_path = cwd.join(exe_path);

                if abs_path.is_dir() {
                    anyhow::bail!("Unable to spawn {} because it is a directory", abs_path.display());
                } else if access(&abs_path, AccessFlags::X_OK).is_ok() {
                    return Ok(abs_path.into_os_string());
                } else if access(&abs_path, AccessFlags::F_OK).is_ok() {
                    anyhow::bail!("Unable to spawn {} because it is not executable", abs_path.display());
                }

                anyhow::bail!("Unable to spawn {} because it does not exist", abs_path.display());
            }

            if let Some(path) = self.resolve_path() {
                for path in std::env::split_paths(&path) {
                    let candidate = cwd.join(&path).join(exe);

                    if candidate.is_dir() {
                        errors.push(format!("{} exists but is a directory", candidate.display()));
                    } else if access(&candidate, AccessFlags::X_OK).is_ok() {
                        return Ok(candidate.into_os_string());
                    } else if access(&candidate, AccessFlags::F_OK).is_ok() {
                        errors.push(format!("{} exists but is not executable", candidate.display()));
                    }
                }
                errors.push(format!("No viable candidates found in PATH {path:?}"));
            } else {
                errors.push("Unable to resolve the PATH".to_string());
            }
            anyhow::bail!("Unable to spawn {} because:\n{}", exe_path.display(), errors.join(".\n"));
        } else if exe_path.is_dir() {
            anyhow::bail!("Unable to spawn {} because it is a directory", exe_path.display());
        } else {
            if let Err(err) = access(exe_path, AccessFlags::X_OK) {
                if access(exe_path, AccessFlags::F_OK).is_ok() {
                    anyhow::bail!("Unable to spawn {} because it is not executable ({err:#})", exe_path.display());
                } else {
                    anyhow::bail!(
                        "Unable to spawn {} because it doesn't exist on the filesystem ({err:#})",
                        exe_path.display()
                    );
                }
            }

            Ok(exe.to_owned())
        }
    }

    /// Convert the CommandBuilder to a `std::process::Command` instance.
    pub(crate) fn as_command(&self) -> anyhow::Result<std::process::Command> {
        use std::os::unix::process::CommandExt;

        let home = self.get_home_dir()?;
        let dir: &OsStr = self.cwd.as_deref().filter(|dir| std::path::Path::new(dir).is_dir()).unwrap_or(home.as_ref());
        let shell = self.get_shell();

        let mut cmd = if self.is_default_prog() {
            let mut cmd = std::process::Command::new(&shell);

            // Run the shell as a login shell by prefixing the shell's
            // basename with `-` and setting that as argv0
            let basename = shell.rsplit('/').next().unwrap_or(&shell);
            cmd.arg0(format!("-{}", basename));
            cmd
        } else {
            let resolved = self.search_path(&self.args[0], dir)?;
            let mut cmd = std::process::Command::new(&resolved);
            cmd.arg0(&self.args[0]);
            cmd.args(&self.args[1..]);
            cmd
        };

        cmd.current_dir(dir);

        cmd.env_clear();
        cmd.env("SHELL", shell);
        cmd.envs(self.envs.values().map(|EnvEntry { is_from_base_env: _, preferred_key, value }| {
            (preferred_key.as_os_str(), value.as_os_str())
        }));

        Ok(cmd)
    }

    /// Determine which shell to run.
    /// We take the contents of the $SHELL env var first, then
    /// fall back to looking it up from the password database.
    pub fn get_shell(&self) -> String {
        use nix::unistd::{AccessFlags, access};

        if let Some(shell) = self.get_env("SHELL").and_then(OsStr::to_str) {
            match access(shell, AccessFlags::X_OK) {
                Ok(()) => return shell.into(),
                Err(err) => log::warn!(
                    "$SHELL -> {shell:?} which is \
                     not executable ({err:#}), falling back to password db lookup"
                ),
            }
        }

        get_shell()
    }

    fn get_home_dir(&self) -> anyhow::Result<String> {
        if let Some(home_dir) = self.get_env("HOME").and_then(OsStr::to_str) {
            return Ok(home_dir.into());
        }

        let ent = unsafe { libc::getpwuid(libc::getuid()) };
        if ent.is_null() {
            Ok("/".into())
        } else {
            use std::ffi::CStr;
            use std::str;
            let home = unsafe { CStr::from_ptr((*ent).pw_dir) };
            home.to_str().map(str::to_owned).context("failed to resolve home dir")
        }
    }
}

#[cfg(windows)]
impl CommandBuilder {
    fn search_path(&self, exe: &OsStr) -> OsString {
        if let Some(path) = self.get_env("PATH") {
            let extensions = self.get_env("PATHEXT").unwrap_or(OsStr::new(".EXE"));
            for path in std::env::split_paths(&path) {
                // Check for exactly the user's string in this path dir
                let candidate = path.join(exe);
                if candidate.exists() {
                    return candidate.into_os_string();
                }

                // otherwise try tacking on some extensions.
                // Note that this really replaces the extension in the
                // user specified path, so this is potentially wrong.
                for ext in std::env::split_paths(&extensions) {
                    // PATHEXT includes the leading `.`, but `with_extension`
                    // doesn't want that
                    let ext = ext.to_str().expect("PATHEXT entries must be utf8");
                    let path = path.join(exe).with_extension(&ext[1..]);
                    if path.exists() {
                        return path.into_os_string();
                    }
                }
            }
        }

        exe.to_owned()
    }

    pub(crate) fn current_directory(&self) -> Option<Vec<u16>> {
        let home: Option<&OsStr> = self.get_env("USERPROFILE").filter(|path| Path::new(path).is_dir());
        let cwd: Option<&OsStr> = self.cwd.as_deref().filter(|path| Path::new(path).is_dir());
        let dir: Option<&OsStr> = cwd.or(home);

        dir.map(|dir| {
            let mut wide = vec![];

            if Path::new(dir).is_relative() {
                if let Ok(ccwd) = std::env::current_dir() {
                    wide.extend(ccwd.join(dir).as_os_str().encode_wide());
                } else {
                    wide.extend(dir.encode_wide());
                }
            } else {
                wide.extend(dir.encode_wide());
            }

            wide.push(0);
            wide
        })
    }

    /// Constructs an environment block for this spawn attempt.
    /// Uses the current process environment as the base and then
    /// adds/replaces the environment that was specified via the
    /// `env` methods.
    pub(crate) fn environment_block(&self) -> Vec<u16> {
        // encode the environment as wide characters
        let mut block = vec![];

        for EnvEntry { is_from_base_env: _, preferred_key, value } in self.envs.values() {
            block.extend(preferred_key.encode_wide());
            block.push(b'=' as u16);
            block.extend(value.encode_wide());
            block.push(0);
        }
        // and a final terminator for CreateProcessW
        block.push(0);

        block
    }

    pub fn get_shell(&self) -> String {
        let exe: OsString = self.get_env("ComSpec").unwrap_or(OsStr::new("cmd.exe")).into();
        exe.into_string().unwrap_or_else(|_| "%CompSpec%".to_string())
    }

    pub(crate) fn cmdline(&self) -> anyhow::Result<(Vec<u16>, Vec<u16>)> {
        let mut cmdline = Vec::<u16>::new();

        let exe: OsString = if self.is_default_prog() {
            self.get_env("ComSpec").unwrap_or(OsStr::new("cmd.exe")).into()
        } else {
            self.search_path(&self.args[0])
        };

        Self::append_quoted(&exe, &mut cmdline);

        // Ensure that we nul terminate the module name, otherwise we'll
        // ask CreateProcessW to start something random!
        let mut exe: Vec<u16> = exe.encode_wide().collect();
        exe.push(0);

        for arg in self.args.iter().skip(1) {
            cmdline.push(' ' as u16);
            anyhow::ensure!(!arg.encode_wide().any(|c| c == 0), "invalid encoding for command line argument {:?}", arg);
            Self::append_quoted(arg, &mut cmdline);
        }
        // Ensure that the command line is nul terminated too!
        cmdline.push(0);
        Ok((exe, cmdline))
    }

    // Borrowed from https://github.com/hniksic/rust-subprocess/blob/873dfed165173e52907beb87118b2c0c05d8b8a1/src/popen.rs#L1117
    // which in turn was translated from ArgvQuote at http://tinyurl.com/zmgtnls
    fn append_quoted(arg: &OsStr, cmdline: &mut Vec<u16>) {
        if !arg.is_empty()
            && !arg.encode_wide().any(|c| {
                c == ' ' as u16 || c == '\t' as u16 || c == '\n' as u16 || c == '\x0b' as u16 || c == '\"' as u16
            })
        {
            cmdline.extend(arg.encode_wide());
            return;
        }
        cmdline.push('"' as u16);

        let arg: Vec<_> = arg.encode_wide().collect();
        let mut i = 0;
        while i < arg.len() {
            let mut num_backslashes = 0;
            while i < arg.len() && arg[i] == '\\' as u16 {
                i += 1;
                num_backslashes += 1;
            }

            if i == arg.len() {
                for _ in 0..num_backslashes * 2 {
                    cmdline.push('\\' as u16);
                }
                break;
            } else if arg[i] == b'"' as u16 {
                for _ in 0..num_backslashes * 2 + 1 {
                    cmdline.push('\\' as u16);
                }
                cmdline.push(arg[i]);
            } else {
                for _ in 0..num_backslashes {
                    cmdline.push('\\' as u16);
                }
                cmdline.push(arg[i]);
            }
            i += 1;
        }
        cmdline.push('"' as u16);
    }
}

#[cfg(unix)]
/// Returns true if the path begins with `./` or `../`
fn is_cwd_relative_path<P: AsRef<Path>>(p: P) -> bool {
    matches!(p.as_ref().components().next(), Some(Component::CurDir | Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn test_cwd_relative() {
        assert!(is_cwd_relative_path("."));
        assert!(is_cwd_relative_path("./foo"));
        assert!(is_cwd_relative_path("../foo"));
        assert!(!is_cwd_relative_path("foo"));
        assert!(!is_cwd_relative_path("/foo"));
    }

    #[test]
    fn test_env() {
        let mut cmd = CommandBuilder::new("dummy");
        let package_authors = cmd.get_env("CARGO_PKG_AUTHORS");
        println!("package_authors: {:?}", package_authors);
        // `cargo test` sets the package variables for the test process; NOBLE's authors, not upstream's.
        assert!(package_authors == Some(OsStr::new(env!("CARGO_PKG_AUTHORS"))));

        cmd.env("foo key", "foo value");
        cmd.env("bar key", "bar value");

        let iterated_envs = cmd.iter_extra_env_as_str().collect::<Vec<_>>();
        println!("iterated_envs: {:?}", iterated_envs);
        assert!(iterated_envs == vec![("bar key", "bar value"), ("foo key", "foo value")]);

        {
            let mut cmd = cmd.clone();
            cmd.env_remove("foo key");

            let iterated_envs = cmd.iter_extra_env_as_str().collect::<Vec<_>>();
            println!("iterated_envs: {:?}", iterated_envs);
            assert!(iterated_envs == vec![("bar key", "bar value")]);
        }

        {
            let mut cmd = cmd.clone();
            cmd.env_remove("bar key");

            let iterated_envs = cmd.iter_extra_env_as_str().collect::<Vec<_>>();
            println!("iterated_envs: {:?}", iterated_envs);
            assert!(iterated_envs == vec![("foo key", "foo value")]);
        }

        {
            let mut cmd = cmd.clone();
            cmd.env_clear();

            let iterated_envs = cmd.iter_extra_env_as_str().collect::<Vec<_>>();
            println!("iterated_envs: {:?}", iterated_envs);
            assert!(iterated_envs.is_empty());
        }
    }

    #[cfg(windows)]
    #[test]
    fn test_env_case_insensitive_override() {
        let mut cmd = CommandBuilder::new("dummy");
        cmd.env("Cargo_Pkg_Authors", "Not Wez");
        assert!(cmd.get_env("cargo_pkg_authors") == Some(OsStr::new("Not Wez")));

        cmd.env_remove("cARGO_pKG_aUTHORS");
        assert!(cmd.get_env("CARGO_PKG_AUTHORS").is_none());
    }
}
