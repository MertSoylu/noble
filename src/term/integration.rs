//! Shell integration for bash, zsh and fish: small startup scripts that load the
//! user's own configuration first and then report the working directory with
//! OSC 7 on every prompt (PowerShell and cmd are handled in `pane.rs`).
//! The OSC 7 report is also the "command finished" signal (`Callbacks::prompt`).
//!
//! The scripts are written to `<data>/shell/` before a pane starts; a file is only
//! rewritten when its content changed.

use std::path::{Path, PathBuf};

/// bash: started with `--rcfile`, which replaces `~/.bashrc`, so it is sourced here.
/// The hook runs last in `PROMPT_COMMAND` and keeps `$?` for prompts that show it.
/// Git Bash (MSYS) and Cygwin show mount points such as `/tmp` or `/usr`: those are turned
/// into Windows paths with prefixes looked up once (`/c/...` is converted by NOBLE).
pub const BASH: &str = r#"# NOBLE shell integration for bash (generated; changes are overwritten).
if [[ -f ~/.bashrc ]]; then builtin source ~/.bashrc; fi
if [[ $OSTYPE == msys* || $OSTYPE == cygwin* ]] && builtin command -v cygpath >/dev/null; then
  __noble_root=$(cygpath -m /) __noble_tmp=$(cygpath -m /tmp)
fi
__noble_osc7() {
  local s=$? p=$PWD
  if [[ -n $__noble_root ]]; then
    case $p in
      /[a-zA-Z] | /[a-zA-Z]/*) ;;
      /cygdrive/*) p=${p#/cygdrive} ;;
      /tmp | /tmp/*) p=/$__noble_tmp${p#/tmp} ;;
      /*) p=/${__noble_root%/}$p ;;
    esac
  fi
  builtin printf '\033]7;file://%s%s\a' "${HOSTNAME}" "${p//\%/%25}"
  return $s
}
if [[ ${PROMPT_COMMAND} != *__noble_osc7* ]]; then
  __noble_nl=$'\n'
  PROMPT_COMMAND="${PROMPT_COMMAND:+${PROMPT_COMMAND}${__noble_nl}}__noble_osc7"
  unset __noble_nl
fi
"#;

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
[[ -f $ZDOTDIR/.zshrc ]] && builtin source $ZDOTDIR/.zshrc
unset __noble_zdotdir
__noble_osc7() { builtin printf '\033]7;file://%s%s\a' "${HOST}" "${PWD//\%/%25}" }
autoload -Uz add-zsh-hook && add-zsh-hook precmd __noble_osc7
"#;

/// fish: loaded with `--init-command` after the user's `config.fish`.
pub const FISH: &str = r#"# NOBLE shell integration for fish (generated; changes are overwritten).
function __noble_osc7 --on-event fish_prompt
    printf '\e]7;file://%s%s\a' $hostname (string replace -a % %25 -- $PWD)
end
"#;

pub fn bash_rc(dir: &Path) -> PathBuf {
    dir.join("bashrc")
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
        assert_eq!(std::fs::read_to_string(zsh_dir(&dir).join(".zshrc")).unwrap(), ZSHRC);
        let before = std::fs::metadata(fish_script(&dir)).unwrap().modified().unwrap();
        install(&dir).unwrap();
        assert_eq!(std::fs::metadata(fish_script(&dir)).unwrap().modified().unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scripts_use_bel_and_escape_percent() {
        // The OSC 7 report ends with BEL and encodes '%' so `parse_cwd_url` decodes it back.
        for script in [BASH, ZSHRC, FISH] {
            assert!(script.contains("]7;file://") && script.contains(r"\a'"), "{script}");
            assert!(script.contains("%25"), "{script}");
        }
    }
}
