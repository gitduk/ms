mod capture;
mod config;
mod restore;
mod tmux;

use std::fs;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::tmux::Run;

#[derive(Parser)]
#[command(version, about = "Save and restore tmux sessions")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Capture a running session into ~/.config/ms/<name>.toml
    Save {
        name: String,
        /// Session to capture [default: the current one]
        #[arg(short, long)]
        session: Option<String>,
        /// Overwrite an existing config
        #[arg(short, long)]
        force: bool,
    },
    /// Start a session from its config (attach if already running)
    Start {
        name: String,
        /// Kill the running session and rebuild it
        #[arg(long)]
        fresh: bool,
        /// Build without attaching
        #[arg(short, long)]
        detached: bool,
    },
    /// List configs; `*` marks running sessions
    Ls,
    /// Open a config in $EDITOR
    Edit { name: String },
    /// Kill a config's session
    Kill { name: String },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Save {
            name,
            session,
            force,
        } => save(&name, session, force),
        Cmd::Start {
            name,
            fresh,
            detached,
        } => restore::start(&Config::load(&name)?, fresh, detached),
        Cmd::Ls => ls(),
        Cmd::Edit { name } => edit(&name),
        Cmd::Kill { name } => {
            let cfg = Config::load(&name)?;
            tmux::cmd()
                .args(["kill-session", "-t", &tmux::exact(&cfg.session)])
                .run()?;
            Ok(())
        }
    }
}

fn save(name: &str, session: Option<String>, force: bool) -> Result<()> {
    let path = config::path_of(name)?;
    config::validate_session(name)?;
    if path.exists() && !force {
        bail!(
            "{} exists (it may hold hand edits); pass --force to overwrite",
            path.display()
        );
    }
    let session = match session {
        Some(s) => s,
        None if tmux::inside() => tmux::current_session()?,
        None => bail!("not inside tmux; pass --session"),
    };
    let cfg = capture::capture(&session, name)?;
    let dir = config::dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    fs::write(&path, toml::to_string_pretty(&cfg)?)
        .with_context(|| format!("cannot write {}", path.display()))?;
    println!("saved {session:?} to {}", path.display());
    Ok(())
}

fn ls() -> Result<()> {
    let dir = config::dir()?;
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(());
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| {
            e.ok()?
                .file_name()
                .into_string()
                .ok()?
                .strip_suffix(".toml")
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    let running = tmux::running_sessions();
    for name in names {
        match Config::load(&name) {
            Ok(cfg) => {
                let mark = if running.contains(&cfg.session) {
                    '*'
                } else {
                    ' '
                };
                println!("{mark} {name}");
            }
            Err(e) => {
                let cause = e.root_cause().to_string();
                println!("! {name}  ({})", cause.lines().next().unwrap_or_default());
            }
        }
    }
    Ok(())
}

fn edit(name: &str) -> Result<()> {
    let path = config::path_of(name)?;
    let dir = config::dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    // Through sh so an $EDITOR with arguments (`code -w`) works.
    let status = Command::new("sh")
        .args(["-c", r#"exec ${EDITOR:-vi} "$1""#, "sh"])
        .arg(&path)
        .status()
        .context("failed to launch editor")?;
    if status.success() && path.exists() {
        Config::load(name)?;
    }
    Ok(())
}
