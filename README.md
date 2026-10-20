# ms

Save and restore tmux sessions as hand-editable TOML.

`ms save` captures a running session's windows, panes, working directories
and commands into `~/.config/ms/<name>.toml`; `ms start` rebuilds it later.

## Install

Download a prebuilt binary from
[Releases](https://github.com/gitduk/ms/releases), or build from source:

```sh
cargo install --git https://github.com/gitduk/ms
```

Zsh completion (config and session names included):

```sh
# ~/.zshrc
eval "$(ms init zsh)"
```

## Usage

```sh
ms save              # capture the current session into <session>.toml
ms save work -e      # capture into work.toml, reviewing it in $EDITOR first
ms ls                # list configs; * marks running sessions
ms view work         # draw work's windows as their pane layouts
ms start work        # build the session (or attach if it's running)
ms start work --fresh  # kill and rebuild it
ms update work       # re-capture only the windows, keeping everything else
ms edit work         # open work.toml in $EDITOR
ms kill work         # kill the session; the config stays
ms rm work           # delete the config; the session stays
```

Inside tmux, `NAME` defaults to the current session's name for `save`,
`update`, `start`, `view` and `edit`. `rm` and `kill` always need it.

`save` refuses to overwrite an existing config, since it may hold hand edits:
pass `--force`, or `-e` to review the new capture first. Hooks can't be
captured from tmux, so `save` keeps the old config's `before` and `after`.

## Config

```toml
session = "work"
root = "~/code/app"          # relative to $HOME; ~ and absolute paths work
before = "docker compose up -d"   # run in the root before a new session is built
after = "echo ready"              # run in the root after it's built

[[windows]]
name = "edit"
panes = [
  { cmd = "nvim" },
  { root = "tests", cmd = "cargo test", enter = false },
]
layout = "main-vertical"

[[windows]]
name = "shell"
root = "/tmp"
```

- `root` nests: a pane's root joins onto its window's, which joins onto the
  session's. Absolute and `~` paths override.
- A window without `panes` opens one shell.
- `enter = false` types `cmd` without pressing Enter.
- `layout` is a tmux preset (`tiled`, `main-vertical`, …) or a
  `#{window_layout}` string, which is what `save` captures.

Configs live in `$XDG_CONFIG_HOME/ms`, falling back to `~/.config/ms`.
