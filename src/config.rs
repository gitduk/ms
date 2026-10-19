use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
    pub windows: Vec<Window>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
    /// A tmux preset (`tiled`, `main-vertical`, ...) or a `#{window_layout}` string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panes: Vec<Pane>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pane {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
    /// `false` types `cmd` into the pane without pressing Enter.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enter: bool,
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

impl Config {
    pub fn load(name: &str) -> Result<Self> {
        let path = path_of(name)?;
        let text =
            fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        let cfg: Self =
            toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))?;
        validate_session(&cfg.session)?;
        ensure!(
            !cfg.windows.is_empty(),
            "{}: no [[windows]] defined",
            path.display()
        );
        Ok(cfg)
    }

    /// Each level's root is joined onto its parent's, so relative paths nest
    /// and absolute (or `~`) paths override.
    pub fn resolve_root(&self, window: &Window, pane: Option<&Pane>) -> Result<PathBuf> {
        let home = home()?;
        let levels = [
            self.root.as_deref(),
            window.root.as_deref(),
            pane.and_then(|p| p.root.as_deref()),
        ];
        Ok(levels
            .into_iter()
            .flatten()
            .fold(home.clone(), |acc, p| acc.join(expand_tilde(p, &home))))
    }
}

pub fn home() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .context("$HOME is not set")
}

fn expand_tilde(p: &Path, home: &Path) -> PathBuf {
    match p.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => p.to_path_buf(),
    }
}

pub fn dir() -> Result<PathBuf> {
    match env::var_os("XDG_CONFIG_HOME") {
        Some(d) if !d.is_empty() => Ok(PathBuf::from(d).join("ms")),
        _ => Ok(home()?.join(".config/ms")),
    }
}

/// Config names (file stems) in the config dir, sorted; empty if it's missing.
pub fn names() -> Vec<String> {
    let Ok(entries) = dir().and_then(|d| Ok(fs::read_dir(d)?)) else {
        return Vec::new();
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
    names
}

pub fn path_of(name: &str) -> Result<PathBuf> {
    ensure!(
        !name.is_empty() && !name.starts_with('.') && !name.contains('/'),
        "invalid config name {name:?}"
    );
    Ok(dir()?.join(format!("{name}.toml")))
}

/// tmux silently rewrites `:` and `.` in session names, which would break targeting.
pub fn validate_session(name: &str) -> Result<()> {
    if name.is_empty() || name.contains([':', '.']) {
        bail!("invalid session name {name:?}: must be non-empty without ':' or '.'");
    }
    Ok(())
}
