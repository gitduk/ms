use std::borrow::Cow;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{self, Config, Pane, Window};
use crate::tmux::{self, Run};

// Free-text fields go through `tmux::raw`, which encodes `|` away.
const SEP: &str = "|";

pub fn capture(session: &str, name: &str) -> Result<Config> {
    let format = [
        "#{window_index}".into(),
        tmux::raw("window_name"),
        "#{window_layout}".into(),
        "#{pane_id}".into(),
        "#{pane_pid}".into(),
        tmux::raw("pane_current_path"),
    ]
    .join(SEP);
    let out = tmux::cmd()
        .args([
            "list-panes",
            "-s",
            "-t",
            &tmux::exact(session),
            "-F",
            &format,
        ])
        .run()?;

    let home = config::home()?;
    // The pane running this save would otherwise be captured as `ms save`.
    let own_pane = env::var("TMUX_PANE").ok();
    let mut windows: Vec<Window> = Vec::new();
    let mut last_index = None;
    for line in out.lines() {
        let mut f = line.splitn(6, SEP);
        let mut next = || f.next().context("unexpected tmux list-panes output");
        let (index, wname, layout) = (next()?, next()?, next()?);
        let (pane_id, pid, path) = (next()?, next()?, next()?);

        let pane = Pane {
            root: Some(tildify(Path::new(&tmux::unvis(path)), &home)),
            cmd: if own_pane.as_deref() == Some(pane_id) {
                None
            } else {
                proc_cmd(pid)
            },
            enter: true,
        };
        match windows.last_mut() {
            Some(w) if last_index == Some(index) => w.panes.push(pane),
            _ => {
                last_index = Some(index);
                windows.push(Window {
                    // Window names are also stored vis-encoded.
                    name: tmux::unvis(&tmux::unvis(wname)),
                    root: None,
                    layout: Some(layout.to_owned()),
                    panes: vec![pane],
                });
            }
        }
    }

    for w in &mut windows {
        w.root = hoist(w.panes.iter_mut().map(|p| &mut p.root));
        if let [only] = w.panes.as_slice()
            && only.cmd.is_none()
        {
            w.panes.clear();
            w.layout = None;
        }
    }
    let root = hoist(windows.iter_mut().map(|w| &mut w.root));

    Ok(Config {
        session: name.to_owned(),
        root,
        windows,
    })
}

/// If every child has the same root, moves it up to the parent.
fn hoist<'a>(children: impl IntoIterator<Item = &'a mut Option<PathBuf>>) -> Option<PathBuf> {
    let mut children: Vec<_> = children.into_iter().collect();
    let first = (**children.first()?).clone()?;
    if !children.iter().all(|r| r.as_ref() == Some(&first)) {
        return None;
    }
    for r in &mut children {
        **r = None;
    }
    Some(first)
}

fn tildify(p: &Path, home: &Path) -> PathBuf {
    match p.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => PathBuf::from("~"),
        Ok(rest) => Path::new("~").join(rest),
        Err(_) => p.to_path_buf(),
    }
}

/// The shell's first child, or the pane process itself when it isn't a
/// shell. `None` for an idle shell.
fn proc_cmd(pid: &str) -> Option<String> {
    let args = cmdline(pid)?;
    let args = if is_shell(&args[0]) {
        let children = fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).ok()?;
        let child = cmdline(children.split_whitespace().next()?)?;
        // A forked subshell (a function, a pipeline) carries the shell's
        // argv, not the command, so there's nothing useful to record.
        if is_shell(&child[0]) {
            return None;
        }
        child
    } else {
        args
    };
    Some(args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "))
}

fn cmdline(pid: &str) -> Option<Vec<String>> {
    let raw = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let raw = raw.strip_suffix(&[0]).unwrap_or(&raw);
    if raw.is_empty() {
        return None;
    }
    Some(
        raw.split(|&b| b == 0)
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect(),
    )
}

fn is_shell(argv0: &str) -> bool {
    let base = argv0
        .rsplit('/')
        .next()
        .unwrap_or(argv0)
        .trim_start_matches('-');
    matches!(base, "sh" | "bash" | "zsh" | "fish" | "dash")
}

fn quote(arg: &str) -> Cow<'_, str> {
    let safe = |c: char| c.is_ascii_alphanumeric() || "-_./=:,@%+".contains(c);
    if !arg.is_empty() && arg.chars().all(safe) {
        Cow::Borrowed(arg)
    } else {
        Cow::Owned(format!("'{}'", arg.replace('\'', r"'\''")))
    }
}
