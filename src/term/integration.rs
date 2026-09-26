//! Shell integration for bash, zsh and fish: small startup scripts that load the
//! user's own configuration first and then report the working directory with
//! OSC 7 on every prompt (PowerShell and cmd are handled in `pane.rs`).
//! The OSC 7 report is also the "command finished" signal (`Callbacks::prompt`).
//!
//! The scripts are written to `<data>/shell/` before a pane starts; a file is only
//! rewritten when its content changed.

use std::path::{Path, PathBuf};

/// The bash prompt hook shared by the rc and the login script. It runs last in
/// `PROMPT_COMMAND` (after the user's own commands) and keeps `$?` for prompts that show it.
/// Git Bash (MSYS) and Cygwin show mount points such as `/tmp` or `/usr`: those are turned
/// into Windows paths with prefixes looked up once (`/c/...` is converted by NOBLE).
/// The path is percent-encoded byte by byte (`LC_ALL=C`), so spaces, `%`, `;` and
/// non-ASCII names survive the round trip through `parse_cwd_url`.
macro_rules! bash_hook {
    () => {
        r#"if [[ $OSTYPE == msys* || $OSTYPE == cygwin* ]] && builtin command -v cygpath >/dev/null; then
  __noble_root=$(cygpath -m /) __noble_tmp=$(cygpath -m /tmp)
fi
__noble_osc7() {
  local s=$? p=$PWD LC_ALL=C u= c i
  if [[ -n $__noble_root ]]; then
    case $p in
      /[a-zA-Z] | /[a-zA-Z]/*) ;;
      /cygdrive/*) p=${p#/cygdrive} ;;
      /tmp | /tmp/*) p=/$__noble_tmp${p#/tmp} ;;
      /*) p=/${__noble_root%/}$p ;;
    esac
  fi
  for (( i = 0; i < ${#p}; i++ )); do
    c=${p:i:1}
    case $c in
      [-/:_.~a-zA-Z0-9]) u+=$c ;;
      *) builtin printf -v c '%%%02X' "'$c"; u+=$c ;;
    esac
  done
  builtin printf '\033]7;file://%s%s\a' "${HOSTNAME}" "$u"
  return $s
}
if [[ ${PROMPT_COMMAND} != *__noble_osc7* ]]; then
  __noble_nl=$'\n'
  PROMPT_COMMAND="${PROMPT_COMMAND:+${PROMPT_COMMAND}${__noble_nl}}__noble_osc7"
  unset __noble_nl
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
[[ -f $ZDOTDIR/.zshrc ]] && builtin source $ZDOTDIR/.zshrc
unset __noble_zdotdir
# The path is percent-encoded byte by byte (`LC_ALL=C`): spaces, `%`, `;` and non-ASCII
# names survive the round trip through NOBLE.
__noble_osc7() {
  emulate -L zsh -o extendedglob
  local LC_ALL=C
  builtin printf '\033]7;file://%s%s\a' "${HOST}" "${PWD//(#m)[^-\/:_.~a-zA-Z0-9]/%${(l:2::0:)$(( [##16] #MATCH ))}}"
}
autoload -Uz add-zsh-hook && add-zsh-hook precmd __noble_osc7
"#;

/// fish: loaded with `--init-command` after the user's `config.fish`.
/// The path is percent-encoded (`string escape --style=url`, fish 3.0+).
pub const FISH: &str = r#"# NOBLE shell integration for fish (generated; changes are overwritten).
function __noble_osc7 --on-event fish_prompt
    printf '\e]7;file://%s%s\a' $hostname (string escape --style=url -- $PWD)
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
            assert!(script.contains("'%%%02X'") && script.contains("PROMPT_COMMAND="), "{script}");
        }
        assert!(ZSHRC.contains("[##16]"));
        assert!(FISH.contains("string escape --style=url"));
    }
}
