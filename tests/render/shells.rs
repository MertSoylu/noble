//! Shell integration edge cases for bash, zsh and fish (Unix only).

use noble::app::App;
use noble::keys::Action;
use std::path::PathBuf;
use std::time::Duration;

use super::common::*;

use noble::term::layout::PaneId;
use std::path::Path;

struct IsolatedShell {
    shell: &'static str,
    home: PathBuf,
    wrapper: PathBuf,
}

impl IsolatedShell {
    /// `None` when the shell is not installed. `env` is exported by the wrapper as well.
    /// `shell` is a name looked up on PATH or a full path (`/bin/bash`).
    fn new(shell: &'static str, case: &str, env: &[(&str, &str)]) -> Option<IsolatedShell> {
        use std::os::unix::fs::PermissionsExt;
        let real = noble::util::which(shell)?;
        let tag = shell.replace('/', "_");
        let root = std::env::temp_dir().join(format!("noble-shell-{case}-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".config").join("fish")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
        let mut script = format!(
            "#!/bin/sh\nexport HOME={h} XDG_CONFIG_HOME={h}/.config XDG_DATA_HOME={h}/.local/share\n\
             unset NOBLE_USER_ZDOTDIR PROMPT_COMMAND\n",
            h = q(&home.display().to_string())
        );
        for (k, v) in env {
            script.push_str(&format!("export {k}={}\n", q(v)));
        }
        script.push_str(&format!("exec {} \"$@\"\n", q(&real.display().to_string())));
        // Named like the shell, so NOBLE recognizes its kind.
        let wrapper = root.join("bin").join(Path::new(shell).file_name().unwrap());
        std::fs::write(&wrapper, script).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        Some(IsolatedShell { shell, home, wrapper })
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.home.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// Opens a tab with this shell and `args` in `cwd`.
    fn open(&self, args: &[&str], cwd: &Path) -> (App, PaneId) {
        let mut app = demo_app(110, 30);
        let mut cfg = app.cfg.clone();
        cfg.terminal.shell = self.wrapper.display().to_string();
        cfg.terminal.shell_args = args.iter().map(|a| a.to_string()).collect();
        app.apply_config(cfg);
        app.new_tab(cwd.to_path_buf(), None, Some("shell".into()));
        let id = app.tabs[0].focus;
        (app, id)
    }
}

impl Drop for IsolatedShell {
    fn drop(&mut self) {
        if let Some(root) = self.home.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}

/// Pumps until the pane's reported directory (OSC 7) is `want`; panics with the screen otherwise.
fn wait_cwd(app: &mut App, id: PaneId, want: &Path, what: &str) {
    let want = std::fs::canonicalize(want).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        app.pump();
        let reported = app.panes[&id].parser().callbacks().cwd.clone();
        // The folder the pane acts on (`Pane::cwd`) follows the report once its check has been applied.
        let acted = app.panes[&id].cwd();
        let acted_ok = std::fs::canonicalize(&acted).ok() == Some(want.clone());
        if acted_ok && reported.as_ref().and_then(|c| std::fs::canonicalize(c).ok()) == Some(want.clone()) {
            return;
        }
        if std::time::Instant::now() > deadline {
            let screen = app.panes[&id].parser().screen().contents();
            panic!("{what}: reported cwd {reported:?}, acted on {acted:?}, want {want:?}\n{screen}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Pumps until the pane's screen contains `needle`.
fn wait_screen(app: &mut App, id: PaneId, needle: &str, what: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        app.pump();
        let screen = app.panes[&id].parser().screen().contents();
        if screen.contains(needle) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "{what}: {needle:?} never appeared\n{screen}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// OSC 7 round trip: directories with spaces, Unicode, `%`, `#`, `;`, `?` and a trailing
/// space are reported by every installed bash/zsh/fish and decoded back to the same path.
#[test]
fn cwd_with_special_characters_is_tracked() {
    for shell in bashes().into_iter().chain(["zsh", "fish"]) {
        let Some(sh) = IsolatedShell::new(shell, "chars", &[]) else { continue };
        let base = sh.home.join("dir with spaces ü 日本");
        let odd = base.join("100% #1;x? ");
        std::fs::create_dir_all(&odd).unwrap();
        let (mut app, id) = sh.open(&[], &base);
        wait_cwd(&mut app, id, &base, &format!("{shell} start"));
        app.panes[&id].write(b"cd '100% #1;x? '\r");
        wait_cwd(&mut app, id, &odd, &format!("{shell} cd"));
        assert_eq!(app.panes[&id].cwd(), odd, "{}", sh.shell);
        app.run(Action::CloseTab);
    }
}

/// bash on PATH, and on macOS also the system `/bin/bash` (3.2), which Homebrew's bash 5
/// usually hides. Linux has one bash.
fn bashes() -> Vec<&'static str> {
    let mut list = vec!["bash"];
    if cfg!(target_os = "macos") && noble::util::which("bash").is_some_and(|b| b != Path::new("/bin/bash")) {
        list.push("/bin/bash");
    }
    list
}

/// `major * 10 + minor` of a bash, e.g. 32 for 3.2 or 52 for 5.2.
fn bash_version(shell: &str) -> u32 {
    let out = std::process::Command::new(noble::util::which(shell).unwrap())
        .args(["-c", "echo $((BASH_VERSINFO[0] * 10 + BASH_VERSINFO[1]))"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
}

/// Types `cd <dir>` and checks both the OSC 7 report and the user's own hook marker.
fn cd_and_check(app: &mut App, id: PaneId, dir: &Path, marker: &str, what: &str) {
    app.panes[&id].write(format!("cd '{}'\r", dir.display()).as_bytes());
    wait_cwd(app, id, dir, what);
    wait_screen(app, id, marker, what);
}

/// The user's own prompt hooks keep running next to NOBLE's, which still reports the cwd.
#[test]
fn user_prompt_hooks_keep_working() {
    let target = std::env::temp_dir().join(format!("noble-hooks-{}", std::process::id()));
    std::fs::create_dir_all(&target).unwrap();
    // bash: a string PROMPT_COMMAND ending in a comment, and the bash 5.1+ array form.
    let cases = [
        ("bash-str", "PROMPT_COMMAND='__u=$((__u+1)); echo \"UHOOK$__u\" # user hook'\n"),
        ("bash-arr", "__u=0\nPROMPT_COMMAND=('__u=$((__u+1))' 'echo \"UHOOK$__u\"')\n"),
    ];
    for bash in bashes() {
        for (case, rc) in cases {
            // Older bash (macOS 3.2) runs only the first element of an array PROMPT_COMMAND.
            if case == "bash-arr" && bash_version(bash) < 51 {
                continue;
            }
            let Some(sh) = IsolatedShell::new(bash, case, &[]) else { break };
            sh.write(".bashrc", rc);
            // macOS starts bash as a login shell, which reads ~/.bash_profile; users source
            // ~/.bashrc from there. (Linux starts it with the rc file only.)
            sh.write(".bash_profile", "[ -f ~/.bashrc ] && . ~/.bashrc\n");
            let (mut app, id) = sh.open(&[], &sh.home);
            wait_cwd(&mut app, id, &sh.home, case);
            wait_screen(&mut app, id, "UHOOK1", case);
            cd_and_check(&mut app, id, &target, "UHOOK2", case);
            app.run(Action::CloseTab);
        }
    }
    if let Some(sh) = IsolatedShell::new("zsh", "hooks", &[]) {
        sh.write(
            ".zshrc",
            "precmd() { __u=$((__u+1)); echo \"UHOOK$__u\" }\n\
             __v() { echo VHOOK }\nprecmd_functions+=(__v)\n",
        );
        let (mut app, id) = sh.open(&[], &sh.home);
        wait_cwd(&mut app, id, &sh.home, "zsh");
        wait_screen(&mut app, id, "UHOOK1", "zsh");
        wait_screen(&mut app, id, "VHOOK", "zsh");
        cd_and_check(&mut app, id, &target, "UHOOK2", "zsh");
        app.run(Action::CloseTab);
    }
    if let Some(sh) = IsolatedShell::new("fish", "hooks", &[]) {
        sh.write(
            ".config/fish/config.fish",
            "set -g __u 0\nfunction fish_prompt\n    set -g __u (math $__u + 1)\n    echo \"UHOOK$__u> \"\nend\n",
        );
        let (mut app, id) = sh.open(&[], &sh.home);
        wait_cwd(&mut app, id, &sh.home, "fish");
        wait_screen(&mut app, id, "UHOOK1>", "fish");
        cd_and_check(&mut app, id, &target, "UHOOK2>", "fish");
        app.run(Action::CloseTab);
    }
    let _ = std::fs::remove_dir_all(&target);
}

/// zsh finds the user's startup files in their own `ZDOTDIR`: one set by `~/.zshenv`
/// (the common `ZDOTDIR=~/.config/zsh` setup) and one NOBLE inherited from its environment
/// (`extra_env` hands it over as `NOBLE_USER_ZDOTDIR`).
#[test]
fn zsh_user_zdotdir_is_sourced() {
    let target = std::env::temp_dir().join(format!("noble-zdotdir-{}", std::process::id()));
    std::fs::create_dir_all(&target).unwrap();
    let Some(sh) = IsolatedShell::new("zsh", "zdot-env", &[]) else { return };
    sh.write(".zshenv", "ZDOTDIR=$HOME/.config/zsh\n");
    sh.write(".config/zsh/.zshrc", "precmd() { echo ZRC_HOOK }\n");
    let (mut app, id) = sh.open(&[], &sh.home);
    wait_cwd(&mut app, id, &sh.home, "zshenv ZDOTDIR");
    wait_screen(&mut app, id, "ZRC_HOOK", "zshenv ZDOTDIR");
    app.panes[&id].write(b"echo \"ZD=$ZDOTDIR\"\r");
    wait_screen(&mut app, id, &format!("ZD={}", sh.home.join(".config/zsh").display()), "zshenv ZDOTDIR");
    cd_and_check(&mut app, id, &target, "ZRC_HOOK", "zshenv ZDOTDIR");
    app.run(Action::CloseTab);
    drop(sh);

    let root = std::env::temp_dir().join(format!("noble-shell-zdot-inherit-zsh-{}", std::process::id()));
    let user = root.join("home").join("zdot");
    let Some(sh) = IsolatedShell::new("zsh", "zdot-inherit", &[("NOBLE_USER_ZDOTDIR", &user.display().to_string())])
    else {
        return;
    };
    sh.write("zdot/.zshenv", "export ZENV_SEEN=1\n");
    sh.write("zdot/.zshrc", "precmd() { echo \"ZRC2_HOOK$ZENV_SEEN\" }\n");
    let (mut app, id) = sh.open(&[], &sh.home);
    wait_cwd(&mut app, id, &sh.home, "inherited ZDOTDIR");
    wait_screen(&mut app, id, "ZRC2_HOOK1", "inherited ZDOTDIR");
    cd_and_check(&mut app, id, &target, "ZRC2_HOOK1", "inherited ZDOTDIR");
    app.run(Action::CloseTab);
    let _ = std::fs::remove_dir_all(&target);
}

/// User shell options: login shells keep the integration (and load their login files);
/// options that skip every startup file start the shell as asked.
#[test]
fn user_shell_args_and_integration() {
    let target = std::env::temp_dir().join(format!("noble-args-{}", std::process::id()));
    std::fs::create_dir_all(&target).unwrap();
    // bash login shells read ~/.bash_profile, not the rc file: NOBLE loads it itself.
    for login in ["-l", "--login"] {
        let Some(sh) = IsolatedShell::new("bash", &format!("login{login}"), &[]) else { break };
        sh.write(".bash_profile", "echo PROFILE_LOADED\n. ~/.bashrc\n");
        sh.write(".bashrc", "PROMPT_COMMAND='echo RC_HOOK'\n");
        let (mut app, id) = sh.open(&[login], &sh.home);
        wait_cwd(&mut app, id, &sh.home, login);
        wait_screen(&mut app, id, "PROFILE_LOADED", login);
        wait_screen(&mut app, id, "RC_HOOK", login);
        cd_and_check(&mut app, id, &target, "RC_HOOK", login);
        app.run(Action::CloseTab);
    }
    // The documented opt-out (README): these start the shell untouched, without the user's
    // files and without NOBLE's hook, and the shell still works.
    for (shell, arg) in
        [("bash", "--norc"), ("zsh", "-f"), ("zsh", "--no-rcs"), ("fish", "--no-config"), ("fish", "-N")]
    {
        let Some(sh) = IsolatedShell::new(shell, &format!("norc{arg}"), &[]) else { continue };
        sh.write(".bashrc", "echo RC_READ\n");
        sh.write(".zshrc", "echo RC_READ\n");
        sh.write(".config/fish/config.fish", "echo RC_READ\n");
        let (mut app, id) = sh.open(&[arg], &sh.home);
        let probe = if shell == "fish" { "echo ALIVE(math 1+1)\r" } else { "echo ALIVE$((1+1))\r" };
        app.panes[&id].write(probe.as_bytes());
        wait_screen(&mut app, id, "ALIVE2", arg);
        let screen = app.panes[&id].parser().screen().contents();
        assert!(!screen.contains("RC_READ"), "{shell} {arg}: rc file was read\n{screen}");
        app.run(Action::CloseTab);
    }
    let _ = std::fs::remove_dir_all(&target);
}
