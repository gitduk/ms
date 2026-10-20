mod capture;
mod config;
mod init;
mod restore;
mod tmux;

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{ArgValueCandidates, CompleteEnv, CompletionCandidate};

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
        /// Config name [default: the captured session's name]
        name: Option<String>,
        /// Session to capture [default: the current one]
        #[arg(short, long, add = ArgValueCandidates::new(session_candidates))]
        session: Option<String>,
        /// Overwrite an existing config
        #[arg(short, long)]
        force: bool,
        /// Review in $EDITOR first; quitting without saving keeps the old config
        #[arg(short, long)]
        edit: bool,
    },
    /// Start a session from its config (attach if already running)
    Start {
        #[arg(add = ArgValueCandidates::new(config_candidates))]
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
    Edit {
        #[arg(add = ArgValueCandidates::new(config_candidates))]
        name: String,
    },
    /// Kill a config's session
    Kill {
        #[arg(add = ArgValueCandidates::new(config_candidates))]
        name: String,
    },
    /// Print shell completion setup
    Init { shell: Shell },
}

#[derive(Clone, ValueEnum)]
enum Shell {
    Zsh,
}

fn config_candidates() -> Vec<CompletionCandidate> {
    config::names()
        .into_iter()
        .map(CompletionCandidate::new)
        .collect()
}

fn session_candidates() -> Vec<CompletionCandidate> {
    tmux::running_sessions()
        .into_iter()
        .map(CompletionCandidate::new)
        .collect()
}

fn main() -> Result<()> {
    // Call back through PATH, not this exe's path: a cached script would
    // otherwise point at a versioned install dir that upgrades remove.
    CompleteEnv::with_factory(Cli::command)
        .completer("ms")
        .complete();
    match Cli::parse().command {
        Cmd::Save {
            name,
            session,
            force,
            edit,
        } => save(name, session, force, edit),
        Cmd::Start {
            name,
            fresh,
            detached,
        } => restore::start(&Config::load(&name)?, fresh, detached),
        Cmd::Ls => ls(),
        Cmd::Init { shell: Shell::Zsh } => init::zsh(),
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

fn save(name: Option<String>, session: Option<String>, force: bool, edit: bool) -> Result<()> {
    let (name, session) = match name {
        Some(n) => (n, session),
        None => {
            let s = target_session(session)?;
            (s.clone(), Some(s))
        }
    };
    let name = name.as_str();
    let path = config::path_of(name)?;
    config::validate_session(name)?;
    let dir = config::dir()?;
    let draft = dir.join(format!(".{name}.save.toml"));
    if edit && draft.exists() {
        println!(
            "resuming the draft from an earlier save (delete it for a fresh capture): {}",
            draft.display()
        );
        return review(&path, &draft, true);
    }
    if path.exists() && !force && !edit {
        bail!(
            "{} exists (it may hold hand edits); pass --force to overwrite, or -e to review first",
            path.display()
        );
    }
    let session = target_session(session)?;
    let cfg = capture::capture(&session, name)?;
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let text = toml::to_string_pretty(&cfg)?;
    if edit {
        fs::write(&draft, text).with_context(|| format!("cannot write {}", draft.display()))?;
        return review(&path, &draft, false);
    }
    fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))?;
    println!("saved {session:?} to {}", path.display());
    Ok(())
}

fn target_session(session: Option<String>) -> Result<String> {
    match session {
        Some(s) => Ok(s),
        None if tmux::inside() => tmux::current_session(),
        None => bail!("not inside tmux; pass --session"),
    }
}

/// Opens `draft`; it replaces `path` only if the editor saved it and it
/// still parses. A resumed draft holds earlier edits, so it's kept on abort.
fn review(path: &Path, draft: &Path, resumed: bool) -> Result<()> {
    let mtime = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
    let before = mtime(draft);
    let saved = open_editor(draft)? && mtime(draft) != before;
    if !saved {
        if resumed {
            println!("not saved; draft kept at {}", draft.display());
        } else {
            fs::remove_file(draft).with_context(|| format!("cannot remove {}", draft.display()))?;
            println!("not saved; {} left unchanged", path.display());
        }
        return Ok(());
    }
    Config::load_path(draft).with_context(|| format!("draft kept at {}", draft.display()))?;
    fs::rename(draft, path).with_context(|| format!("cannot write {}", path.display()))?;
    println!("saved to {}", path.display());
    Ok(())
}

/// Runs $EDITOR on `path`; returns whether it exited successfully.
fn open_editor(path: &Path) -> Result<bool> {
    // Through sh so an $EDITOR with arguments (`code -w`) works.
    let status = Command::new("sh")
        .args(["-c", r#"exec ${EDITOR:-vi} "$1""#, "sh"])
        .arg(path)
        .status()
        .context("failed to launch editor")?;
    Ok(status.success())
}

fn ls() -> Result<()> {
    let running = tmux::running_sessions();
    for name in config::names() {
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
    if open_editor(&path)? && path.exists() {
        Config::load(name)?;
    }
    Ok(())
}
