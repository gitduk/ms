mod capture;
mod config;
mod init;
mod restore;
mod tmux;
mod view;

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{ArgValueCandidates, CompleteEnv, CompletionCandidate};
use toml_edit::DocumentMut;

use crate::config::{Config, Window};
use crate::tmux::Run;

#[derive(Parser)]
#[command(version, about = "Save and restore tmux sessions")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Capture a running session into ~/.config/ms/<session>.toml; an existing
    /// config gets only its windows replaced, the rest stays as written
    Save {
        /// Session to capture [default: the current one]
        #[arg(short, long, add = ArgValueCandidates::new(session_candidates))]
        session: Option<String>,
        /// Rewrite an existing config whole instead of only its windows
        #[arg(short, long)]
        force: bool,
        /// Review in $EDITOR first; quitting without saving keeps the old config
        #[arg(short, long)]
        edit: bool,
    },
    /// Start a session from its config (attach if already running)
    Start {
        /// Config name [default: the current session's name]
        #[arg(add = ArgValueCandidates::new(config_candidates))]
        name: Option<String>,
        /// Kill the running session and rebuild it
        #[arg(long)]
        fresh: bool,
        /// Build without attaching
        #[arg(short, long)]
        detached: bool,
    },
    /// List configs; `*` marks running sessions
    Ls,
    /// Preview a config's windows as their pane layouts
    View {
        /// Config name [default: the current session's name]
        #[arg(add = ArgValueCandidates::new(config_candidates))]
        name: Option<String>,
    },
    /// Open a config in $EDITOR
    Edit {
        /// Config name [default: the current session's name]
        #[arg(add = ArgValueCandidates::new(config_candidates))]
        name: Option<String>,
    },
    /// Delete a config; its running session is left alone
    Rm {
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
            session,
            force,
            edit,
        } => save(session, force, edit),
        Cmd::Start {
            name,
            fresh,
            detached,
        } => restore::start(&Config::load(&config_name(name, None)?)?, fresh, detached),
        Cmd::Ls => ls(),
        Cmd::View { name } => view::print(&Config::load(&config_name(name, None)?)?),
        Cmd::Init { shell: Shell::Zsh } => init::zsh(),
        Cmd::Edit { name } => edit(&config_name(name, None)?),
        Cmd::Rm { name } => {
            let path = config::path_of(&name)?;
            fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))?;
            println!("removed {}", path.display());
            Ok(())
        }
        Cmd::Kill { name } => {
            let cfg = Config::load(&name)?;
            tmux::cmd()
                .args(["kill-session", "-t", &tmux::exact(&cfg.session)])
                .run()?;
            Ok(())
        }
    }
}

fn save(session: Option<String>, force: bool, edit: bool) -> Result<()> {
    let session = target_session(session)?;
    let name = session.as_str();
    config::validate_session(name)?;
    let path = config::path_of(name)?;
    let dir = config::dir()?;
    let draft = dir.join(format!(".{name}.save.toml"));
    if edit && draft.exists() {
        println!(
            "resuming the draft from an earlier save (delete it for a fresh capture): {}",
            draft.display()
        );
        return review(&path, &draft, true);
    }
    let cfg = capture::capture(&session, name)?;
    let exists = path.exists();
    let text = if exists && !force {
        replace_windows(&path, cfg)?
    } else {
        whole(&path, cfg)?
    };
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    if edit {
        fs::write(&draft, text).with_context(|| format!("cannot write {}", draft.display()))?;
        return review(&path, &draft, false);
    }
    fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))?;
    if exists && !force {
        println!("updated the windows of {} from {session:?}", path.display());
    } else {
        println!("saved {session:?} to {}", path.display());
    }
    Ok(())
}

/// The existing config at `path` with its windows swapped for `cfg`'s;
/// session, root, hooks and comments stay as written.
fn replace_windows(path: &Path, mut cfg: Config) -> Result<String> {
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let old = Config::parse(&text, path)
        .context("can't update its windows; pass --force to rewrite it whole")?;
    let mut doc: DocumentMut = text.parse()?;
    // The old session root stays, so a different captured one moves down
    // to the windows that relied on it.
    if cfg.root != old.root {
        for w in &mut cfg.windows {
            if w.root.is_none() && w.panes.iter().all(|p| p.root.is_none()) {
                w.root = cfg.root.clone();
            }
        }
    }
    doc["windows"] = cfg
        .to_doc()?
        .remove("windows")
        .context("no windows captured")?;
    let text = doc.to_string();
    toml::from_str::<Config>(&text).context("updated config doesn't parse")?;
    Ok(text)
}

/// `cfg` as a whole config, keeping the hooks of any old one at `path`.
fn whole(path: &Path, mut cfg: Config) -> Result<String> {
    // Hooks can't be captured from tmux; keep the ones already written.
    if path.exists() {
        match Config::load_path(path) {
            Ok(old) => {
                cfg.before = old.before;
                cfg.after = old.after;
            }
            Err(e) => eprintln!("warning: hooks not kept, old config unreadable: {e:#}"),
        }
    }
    cfg.to_toml()
}

/// A config is named after its session unless named explicitly.
fn config_name(name: Option<String>, session: Option<String>) -> Result<String> {
    match name {
        Some(n) => Ok(n),
        None => target_session(session),
    }
}

fn target_session(session: Option<String>) -> Result<String> {
    match session {
        Some(s) => Ok(s),
        None if tmux::inside() => tmux::current_session(),
        None => bail!("not inside tmux; name the config or session explicitly"),
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
    let configs: Vec<_> = config::names()
        .into_iter()
        .map(|n| {
            let cfg = Config::load(&n);
            (n, cfg)
        })
        .collect();
    let width = configs
        .iter()
        .map(|(n, _)| n.chars().count())
        .max()
        .unwrap_or(0);
    let windows = |cfg: &Config| view::count(cfg.windows.len(), "window");
    let wwidth = configs
        .iter()
        .filter_map(|(_, c)| Some(windows(c.as_ref().ok()?).len()))
        .max()
        .unwrap_or(0);
    for (name, cfg) in configs {
        match cfg {
            Ok(cfg) => {
                let mark = if running.contains(&cfg.session) {
                    '*'
                } else {
                    ' '
                };
                let panes = cfg.windows.iter().map(Window::pane_count).sum();
                println!(
                    "{mark} {name:<width$}  {:<wwidth$} · {}",
                    windows(&cfg),
                    view::count(panes, "pane")
                );
            }
            Err(e) => {
                let cause = e.root_cause().to_string();
                let cause = cause.lines().next().unwrap_or_default();
                println!("! {name:<width$}  ({cause})");
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
