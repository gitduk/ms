use std::env;
use std::ffi::OsStr;
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
        let prog = self.get_program().to_string_lossy().into_owned();
        let out = self
            .output()
            .with_context(|| format!("failed to run {prog}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            match stderr.trim() {
                "" => bail!("{prog} exited with {}", out.status),
                msg => bail!("{prog}: {msg}"),
            }
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_owned())
    }
}

/// `=name` matches exactly; a bare `prod` would also match `production`.
pub fn exact(session: &str) -> String {
    format!("={session}")
}

/// Escapes `#` for arguments tmux format-expands (names, `-c` paths).
pub fn literal(s: impl AsRef<OsStr>) -> String {
    s.as_ref().to_string_lossy().replace('#', "##")
}

/// A format for `field` whose output `unvis` decodes exactly and holds no
/// `|`: backslashes are doubled (3.4–3.5 output escaping leaves them bare),
/// and `|` becomes `\174`.
pub fn raw(field: &str) -> String {
    format!(r"#{{s/[|]/\\174/:#{{s/\\/\\\\/:{field}}}}}")
}

/// Decodes tmux's vis(3) escaping: window names are stored encoded, and
/// 3.4–3.5 also encode all command output, adding `\$` before variables.
pub fn unvis(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 == b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let octal = b
            .get(i + 1..i + 4)
            .filter(|d| matches!(d, [b'0'..=b'3', b'0'..=b'7', b'0'..=b'7']))
            .map(|d| d.iter().fold(0, |n, c| n * 8 + (c - b'0')));
        if let Some(byte) = octal {
            out.push(byte);
            i += 4;
            continue;
        }
        out.push(match b[i + 1] {
            b'n' => b'\n',
            b't' => b'\t',
            b'r' => b'\r',
            b'a' => 0x07,
            b'b' => 0x08,
            b'v' => 0x0b,
            b'f' => 0x0c,
            b's' => b' ',
            c => c,
        });
        i += 2;
    }
    String::from_utf8_lossy(&out).into_owned()
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
