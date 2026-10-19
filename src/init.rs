use std::io::{self, Write};

use anyhow::Result;
use clap_complete::env::{EnvCompleter, Zsh};

/// Pane option holding the command line the user typed, set by the zsh hook.
pub const CMD_OPTION: &str = "@ms_cmd";

pub fn zsh() -> Result<()> {
    let mut out = io::stdout().lock();
    write!(
        out,
        r#"if [[ -n $TMUX_PANE ]]; then
  _ms_preexec() {{ tmux set-option -pt "$TMUX_PANE" -- {CMD_OPTION} "$1" 2>/dev/null; _ms_cmd_set=1 }}
  _ms_precmd() {{ (( _ms_cmd_set )) && tmux set-option -put "$TMUX_PANE" {CMD_OPTION} 2>/dev/null; _ms_cmd_set=0 }}
  autoload -Uz add-zsh-hook
  add-zsh-hook preexec _ms_preexec
  add-zsh-hook precmd _ms_precmd
fi
"#
    )?;
    Zsh.write_registration("COMPLETE", "ms", "ms", "ms", &mut out)?;
    Ok(())
}
