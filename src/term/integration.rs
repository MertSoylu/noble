//! Shell integration for bash, zsh and fish: small startup scripts that load the
//! user's own configuration first and then report, on every prompt, the last command's
//! exit code (OSC 133;D;<code>) and the working directory (OSC 7). PowerShell and cmd are
//! handled in `pane.rs`. The reports are also the "command finished" signal
//! (`Callbacks::prompt`). Before a typed command line runs they send OSC 133;C (bash 4.4+
//! `PS0`, zsh `preexec`, fish `fish_preexec`): only that starts NOBLE's clock and records a
//! result, never an empty line or a continuation line.
//!
//! The scripts are written to `<data>/shell/` before a pane starts; a file is only
//! rewritten when its content changed.

use std::path::{Path, PathBuf};

/// The bash prompt hook shared by the rc and the login script. `__noble_status` runs first in
/// `PROMPT_COMMAND` and saves the command's `$?` before the user's own prompt commands can change
/// it; `__noble_osc7` runs last and reports it (OSC 133;D) with the directory. Both return the
/// status they found, so `$?` is kept for prompts that show it.
/// Git Bash (MSYS) and Cygwin show mount points such as `/tmp` or `/usr`: those are turned
/// into Windows paths with prefixes looked up once (`/c/...` is converted by NOBLE).
/// The path is percent-encoded byte by byte (`LC_ALL=C`), so spaces, `%`, `;` and
/// non-ASCII names survive the round trip through `parse_cwd_url`. The encoding runs in a
/// subshell (`$(__noble_urlenc …)`), only for paths that need it: the C locale can never leak
/// into the interactive shell. It sticks to bash 3.2 syntax (macOS `/bin/bash`): no
/// `printf -v`, no `case` written inside `$(…)` (3.2 cannot parse it), and a byte's value is
/// masked (3.2 gives bytes above 0x7F as negative numbers).
macro_rules! bash_hook {
    () => {
        r#"if [[ $OSTYPE == msys* || $OSTYPE == cygwin* ]] && builtin command -v cygpath >/dev/null; then
  __noble_root=$(cygpath -m /) __noble_tmp=$(cygpath -m /tmp)
fi
__noble_urlenc() {
  LC_ALL=C
  local i=0 c
  while (( i < ${#1} )); do
    c=${1:i:1}
    case $c in
      [-/:_.~a-zA-Z0-9]) builtin printf '%s' "$c" ;;
      # bash 3.2 gives bytes above 0x7F as negative numbers: keep the low byte.
      *) builtin printf '%%%02X' $(( $(builtin printf '%d' "'$c") & 255 )) ;;
    esac
    (( i++ ))
  done
}
__noble_status() {
  __noble_s=$?
  return $__noble_s
}
__noble_osc7() {
  local s=$? p=$PWD u
  if [[ -n $__noble_root ]]; then
    case $p in
      /[a-zA-Z] | /[a-zA-Z]/*) ;;
      /cygdrive/*) p=${p#/cygdrive} ;;
      /tmp | /tmp/*) p=/$__noble_tmp${p#/tmp} ;;
      /*) p=/${__noble_root%/}$p ;;
    esac
  fi
  if [[ $p == *[!-/:_.~a-zA-Z0-9]* ]]; then u=$(__noble_urlenc "$p"); else u=$p; fi
  builtin printf '\033]133;D;%s\a\033]7;file://%s%s\a' "${__noble_s:-$s}" "${HOSTNAME}" "$u"
  return $s
}
if [[ ${PROMPT_COMMAND} != *__noble_osc7* ]]; then
  __noble_nl=$'\n'
  PROMPT_COMMAND="__noble_status${__noble_nl}${PROMPT_COMMAND:+${PROMPT_COMMAND}${__noble_nl}}__noble_osc7"
  unset __noble_nl
fi
# bash 4.4+ prints PS0 after reading a command line and before running it (never for an empty
# line): it marks the command's start (OSC 133;C). bash 3.2 ignores it; NOBLE then times from Enter.
if [[ ${PS0} != *']133;C'* ]]; then
  PS0=$'\033]133;C\a'"${PS0}"
fi
"#
    };
}

/// bash: started with `--rcfile`, which replaces `~/.bashrc`, so it is sourced here.
pub const BASH: &str = concat!(
    "# NOBLE shell integration for bash (generated; changes are overwritten).\n",
    "if [[ -f ~/.bashrc ]]; then builtin source ~/.bashrc; fi\n",
    bash_hook!()
);

/// bash login shell (`-l`/`--login` among the user's arguments): a login shell never reads
/// the rc file, so NOBLE starts it without the option and this script loads the login files
/// the way bash does: `/etc/profile`, then the first of `~/.bash_profile`, `~/.bash_login`
/// and `~/.profile`.
pub const BASH_LOGIN: &str = concat!(
    "# NOBLE shell integration for bash login shells (generated; changes are overwritten).\n",
    "if [[ -r /etc/profile ]]; then builtin source /etc/profile; fi\n",
    "for __noble_f in ~/.bash_profile ~/.bash_login ~/.profile; do\n",
    "  if [[ -r $__noble_f ]]; then builtin source \"$__noble_f\"; break; fi\n",
    "done\n",
    "unset __noble_f\n",
    bash_hook!()
);

/// zsh `.zshenv`: `ZDOTDIR` points at the NOBLE directory while zsh starts; every
/// file sources the user's own copy from `NOBLE_USER_ZDOTDIR` (default `$HOME`).
pub const ZSHENV: &str = r#"# NOBLE shell integration for zsh (generated; changes are overwritten).
__noble_zdotdir=$ZDOTDIR
ZDOTDIR=${NOBLE_USER_ZDOTDIR:-$HOME}
[[ -f $ZDOTDIR/.zshenv ]] && builtin source $ZDOTDIR/.zshenv
# The user's .zshenv may move ZDOTDIR itself.
export NOBLE_USER_ZDOTDIR=$ZDOTDIR
ZDOTDIR=$__noble_zdotdir
"#;

pub const ZPROFILE: &str = r#"# NOBLE shell integration for zsh (generated; changes are overwritten).
ZDOTDIR=$NOBLE_USER_ZDOTDIR
[[ -f $ZDOTDIR/.zprofile ]] && builtin source $ZDOTDIR/.zprofile
ZDOTDIR=$__noble_zdotdir
"#;

/// zsh `.zshrc`: after it `ZDOTDIR` stays the user's, so `.zlogin`, completion
/// dumps and nested shells behave as without NOBLE.
pub const ZSHRC: &str = r#"# NOBLE shell integration for zsh (generated; changes are overwritten).
ZDOTDIR=$NOBLE_USER_ZDOTDIR
# macOS /etc/zshrc sets HISTFILE=${ZDOTDIR:-$HOME}/.zsh_history while ZDOTDIR is still
# NOBLE's directory; the history belongs in the user's own file. (Linux has no such line.)
[[ $HISTFILE == $__noble_zdotdir/.zsh_history ]] && HISTFILE=$ZDOTDIR/.zsh_history
[[ -f $ZDOTDIR/.zshrc ]] && builtin source $ZDOTDIR/.zshrc
unset __noble_zdotdir
# The last command's exit code goes first (OSC 133;D): `$?` is read before anything resets it
# (every precmd function starts with the command's own status). The path is percent-encoded
# byte by byte (`LC_ALL=C`): spaces, `%`, `;` and non-ASCII names survive the round trip.
__noble_osc7() {
  local s=$?
  emulate -L zsh -o extendedglob
  local LC_ALL=C
  builtin printf '\033]133;D;%s\a' "$s"
  builtin printf '\033]7;file://%s%s\a' "${HOST}" "${PWD//(#m)[^-\/:_.~a-zA-Z0-9]/%${(l:2::0:)$(( [##16] #MATCH ))}}"
}
# A typed command line is about to run (preexec is not called for an empty line): OSC 133;C.
__noble_preexec() {
  builtin printf '\033]133;C\a'
}
autoload -Uz add-zsh-hook && add-zsh-hook precmd __noble_osc7 && add-zsh-hook preexec __noble_preexec
"#;

/// fish: loaded with `--init-command` after the user's `config.fish`. The exit code is read
/// first (an event handler starts with the command's `$status`) and reported with OSC 133;D;
/// the path is percent-encoded (`string escape --style=url`, fish 3.0+). `fish_preexec` (not
/// sent for an empty line) marks a command's start with OSC 133;C.
pub const FISH: &str = r#"# NOBLE shell integration for fish (generated; changes are overwritten).
function __noble_osc7 --on-event fish_prompt
    set -l s $status
    printf '\e]133;D;%s\a' $s
    printf '\e]7;file://%s%s\a' $hostname (string escape --style=url -- $PWD)
end
function __noble_preexec --on-event fish_preexec
    printf '\e]133;C\a'
end
"#;

pub fn bash_rc(dir: &Path) -> PathBuf {
    dir.join("bashrc")
}

pub fn bash_login_rc(dir: &Path) -> PathBuf {
    dir.join("bash_login")
}

pub fn zsh_dir(dir: &Path) -> PathBuf {
    dir.join("zsh")
}

pub fn fish_script(dir: &Path) -> PathBuf {
    dir.join("noble.fish")
}

/// Writes every script under `dir`; files that are already current are not touched.
pub fn install(dir: &Path) -> std::io::Result<()> {
    let zsh = zsh_dir(dir);
    std::fs::create_dir_all(&zsh)?;
    let files = [
        (bash_rc(dir), BASH),
        (bash_login_rc(dir), BASH_LOGIN),
        (zsh.join(".zshenv"), ZSHENV),
        (zsh.join(".zprofile"), ZPROFILE),
        (zsh.join(".zshrc"), ZSHRC),
        (fish_script(dir), FISH),
    ];
    for (path, text) in files {
        if std::fs::read_to_string(&path).is_ok_and(|current| current == text) {
            continue;
        }
        std::fs::write(&path, text)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_writes_once() {
        let dir = std::env::temp_dir().join(format!("noble-integration-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        install(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(bash_rc(&dir)).unwrap(), BASH);
        assert_eq!(std::fs::read_to_string(bash_login_rc(&dir)).unwrap(), BASH_LOGIN);
        assert_eq!(std::fs::read_to_string(zsh_dir(&dir).join(".zshrc")).unwrap(), ZSHRC);
        let before = std::fs::metadata(fish_script(&dir)).unwrap().modified().unwrap();
        install(&dir).unwrap();
        assert_eq!(std::fs::metadata(fish_script(&dir)).unwrap().modified().unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scripts_use_bel_and_percent_encode() {
        // The OSC 7 report ends with BEL and percent-encodes the path so `parse_cwd_url`
        // decodes it back (the real round trip runs in `tests/render.rs`).
        for script in [BASH, BASH_LOGIN, ZSHRC, FISH] {
            assert!(script.contains("]7;file://") && script.contains(r"\a'"), "{script}");
        }
        for script in [BASH, BASH_LOGIN] {
            assert!(
                script.contains("'%%%02X'") && script.contains("& 255") && script.contains("PROMPT_COMMAND="),
                "{script}"
            );
        }
        assert!(ZSHRC.contains("[##16]"));
        assert!(FISH.contains("string escape --style=url"));
    }

    /// Every script reports the last command's exit code (OSC 133;D;<code>) before the directory,
    /// reading it before anything else runs. (`tests/render.rs` runs a failing command for real.)
    #[test]
    fn scripts_report_the_exit_code_first() {
        for script in [BASH, BASH_LOGIN, ZSHRC, FISH] {
            let d = script.find("]133;D;%s").unwrap_or_else(|| panic!("no 133;D in {script}"));
            assert!(d < script.find("]7;file://").unwrap(), "{script}");
        }
        for script in [BASH, BASH_LOGIN] {
            // `$?` is saved by a hook that runs before the user's own PROMPT_COMMAND, and handed on.
            assert!(script.contains("__noble_s=$?\n  return $__noble_s"), "{script}");
            assert!(script.contains(r#"PROMPT_COMMAND="__noble_status${__noble_nl}"#), "{script}");
            assert!(script.contains(r#""${__noble_s:-$s}""#), "{script}");
            // bash 3.2: no `printf -v`, no `local -n`, no `${var@…}`.
            assert!(!script.contains("printf -v") && !script.contains("local -n") && !script.contains("@Q}"));
        }
        let zsh = &ZSHRC[ZSHRC.find("__noble_osc7() {").unwrap()..];
        assert!(zsh.starts_with("__noble_osc7() {\n  local s=$?\n"), "{zsh}");
        let fish = &FISH[FISH.find("--on-event fish_prompt").unwrap()..];
        assert!(fish.starts_with("--on-event fish_prompt\n    set -l s $status\n"), "{fish}");
    }

    /// Every script marks a typed command's start (OSC 133;C) from a hook that only runs for a
    /// command line that is not empty. (`tests/render.rs` checks it for real where the shell exists.)
    #[test]
    fn scripts_mark_command_starts() {
        for script in [BASH, BASH_LOGIN] {
            assert!(script.contains(r#"PS0=$'\033]133;C\a'"${PS0}""#), "{script}");
            // Sourcing the script twice does not add a second mark.
            assert!(script.contains("if [[ ${PS0} != *']133;C'* ]]; then"), "{script}");
        }
        assert!(ZSHRC.contains("add-zsh-hook preexec __noble_preexec") && ZSHRC.contains(r"'\033]133;C\a'"));
        assert!(FISH.contains("--on-event fish_preexec\n    printf '\\e]133;C\\a'"), "{FISH}");
    }
}
