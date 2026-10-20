use std::io::{self, Write};

use anyhow::Result;
use clap_complete::env::{EnvCompleter, Zsh};

pub fn zsh() -> Result<()> {
    let mut out = io::stdout().lock();
    write!(
        out,
        r#"autoload -Uz add-zsh-hook
_ms_complete_register() {{
"#
    )?;
    Zsh.write_registration("COMPLETE", "ms", "ms", "ms", &mut out)?;
    // This may be sourced before compinit; then register at the first prompt,
    // when the rest of .zshrc has run.
    write!(
        out,
        r#"}}
if (( $+functions[compdef] )); then
  _ms_complete_register
else
  _ms_complete_deferred() {{
    add-zsh-hook -d precmd _ms_complete_deferred
    (( $+functions[compdef] )) && _ms_complete_register
  }}
  add-zsh-hook precmd _ms_complete_deferred
fi
"#
    )?;
    Ok(())
}
