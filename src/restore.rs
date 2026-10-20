use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};

use crate::config::{Config, Pane};
use crate::tmux::{self, Run};

pub fn start(cfg: &Config, fresh: bool, detached: bool) -> Result<()> {
    let target = tmux::exact(&cfg.session);
    if tmux::has_session(&cfg.session) {
        if !fresh {
            return if detached {
                Ok(())
            } else {
                tmux::attach(&cfg.session)
            };
        }
        if tmux::inside() && tmux::current_session()? == cfg.session {
            bail!(
                "refusing --fresh from inside {:?}: it would kill this shell",
                cfg.session
            );
        }
        tmux::cmd().args(["kill-session", "-t", &target]).run()?;
    }
    let root = cfg.resolve_root(None, None)?;
    if let Some(script) = &cfg.before {
        hook("before", script, &root)?;
    }
    build(cfg)?;
    if let Some(script) = &cfg.after {
        hook("after", script, &root)
            .with_context(|| format!("session {:?} was built", cfg.session))?;
    }
    if detached {
        Ok(())
    } else {
        tmux::attach(&cfg.session)
    }
}

fn build(cfg: &Config) -> Result<()> {
    let target = tmux::exact(&cfg.session);

    // Resolve and check every directory before touching tmux, so a typo
    // fails cleanly instead of leaving a half-built session behind.
    let mut plan = Vec::with_capacity(cfg.windows.len());
    for w in &cfg.windows {
        let roots = if w.panes.is_empty() {
            vec![cfg.resolve_root(Some(w), None)?]
        } else {
            w.panes
                .iter()
                .map(|p| cfg.resolve_root(Some(w), Some(p)))
                .collect::<Result<Vec<PathBuf>>>()?
        };
        for r in &roots {
            ensure!(
                r.is_dir(),
                "window {:?}: {} is not a directory",
                w.name,
                r.display()
            );
        }
        plan.push((w, roots));
    }

    let mut typed: Vec<(String, &Pane)> = Vec::new();
    for (i, (w, roots)) in plan.into_iter().enumerate() {
        let mut create = tmux::cmd();
        if i == 0 {
            create.args(["new-session", "-d", "-s", &tmux::literal(&cfg.session)]);
            if let Some((cols, rows)) = terminal_size() {
                create.args(["-x", &cols.to_string(), "-y", &rows.to_string()]);
            }
        } else {
            create.args(["new-window", "-d", "-t", &format!("{target}:")]);
        }
        let first = create
            .args(["-P", "-F", "#{pane_id}", "-c"])
            .arg(tmux::literal(&roots[0]))
            .run()?;
        // Not `-n`: 3.4 stores that name raw but rename-window's vis-encoded,
        // and capture can only decode one form.
        tmux::cmd()
            .args(["rename-window", "-t", &first, &tmux::literal(&w.name)])
            .run()?;

        // Split from the newest pane: tmux inserts after the target, so this
        // keeps pane order equal to config order, which layouts fill by.
        let mut ids = vec![first];
        for root in &roots[1..] {
            let id = tmux::cmd()
                .args([
                    "split-window",
                    "-d",
                    "-t",
                    &ids[ids.len() - 1],
                    "-P",
                    "-F",
                    "#{pane_id}",
                    "-c",
                ])
                .arg(tmux::literal(root))
                .run()?;
            ids.push(id);
            // Rebalance after each split; repeatedly halving one pane runs out of room.
            tmux::cmd()
                .args(["select-layout", "-t", &ids[0], "tiled"])
                .run()?;
        }

        let layout = w.layout.as_deref().unwrap_or("tiled");
        if let Err(e) = tmux::cmd()
            .args(["select-layout", "-t", &ids[0], layout])
            .run()
        {
            eprintln!(
                "warning: window {:?}: layout not applied, using tiled: {e:#}",
                w.name
            );
        }

        typed.extend(ids.into_iter().zip(&w.panes));
    }

    // Typed last so each shell has had the whole build to start up.
    for (id, pane) in typed {
        let Some(cmd) = &pane.cmd else { continue };
        // -l sends text literally, so a word like "Enter" in cmd stays text.
        tmux::cmd()
            .args(["send-keys", "-t", &id, "-l", cmd])
            .run()?;
        if pane.enter {
            tmux::cmd().args(["send-keys", "-t", &id, "Enter"]).run()?;
        }
    }

    tmux::cmd()
        .args(["select-window", "-t", &format!("{target}:^")])
        .run()?;
    Ok(())
}

/// `-e` stops a multi-line script at its first failing command.
fn hook(name: &str, script: &str, dir: &Path) -> Result<()> {
    ensure!(dir.is_dir(), "{name}: {} is not a directory", dir.display());
    let status = Command::new("sh")
        .args(["-ec", script])
        .current_dir(dir)
        .status()
        .with_context(|| format!("failed to run {name}"))?;
    ensure!(status.success(), "{name} exited with {status}");
    Ok(())
}

/// The terminal we'll attach from. Without it a detached session starts at
/// tmux's 80x24 default, and presets like main-vertical squash the rest.
fn terminal_size() -> Option<(u16, u16)> {
    let out = Command::new("stty")
        .arg("size")
        .stdin(Stdio::inherit())
        .stderr(Stdio::null())
        .run()
        .ok()?;
    let mut it = out.split_whitespace().map(str::parse::<u16>);
    let rows = it.next()?.ok()?;
    let cols = it.next()?.ok()?;
    Some((cols, rows))
}
