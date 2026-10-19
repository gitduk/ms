use std::env;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

pub fn cmd() -> Command {
    Command::new("tmux")
}

pub trait Run {
    /// Runs to completion and returns trimmed stdout; non-zero exit is an error.
    fn run(&mut self) -> Result<String>;
}

impl Run for Command {
    fn run(&mut self) -> Result<String> {
        let out = self
            .output()
            .with_context(|| format!("failed to spawn {self:?}"))?;
        if !out.status.success() {
            bail!("{self:?}: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_owned())
    }
}

/// `=name` matches exactly; a bare `prod` would also match `production`.
pub fn exact(session: &str) -> String {
    format!("={session}")
}

pub fn has_session(session: &str) -> bool {
    cmd()
        .args(["has-session", "-t", &exact(session)])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn inside() -> bool {
    env::var_os("TMUX").is_some()
}

pub fn current_session() -> Result<String> {
    cmd()
        .args(["display-message", "-p", "#{session_name}"])
        .run()
}

pub fn running_sessions() -> Vec<String> {
    cmd()
        .args(["list-sessions", "-F", "#{session_name}"])
        .stderr(Stdio::null())
        .run()
        .map(|out| out.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Replaces this process with the tmux client; only returns on failure.
pub fn attach(session: &str) -> Result<()> {
    let sub = if inside() {
        "switch-client"
    } else {
        "attach-session"
    };
    let err = cmd().args([sub, "-t", &exact(session)]).exec();
    Err(err).context("failed to exec tmux")
}
