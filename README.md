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
ms save -s work      # capture the running session work into work.toml
ms save -e           # the same as `ms save`, reviewing it in $EDITOR first
ms save -f           # rewrite the config whole instead of only its windows
ms ls                # list configs with window/pane counts; * marks running
ms view work         # draw work's windows as their pane layouts
ms start work        # build the session (or attach if it's running)
ms start work -d     # build it without attaching
ms start work --fresh  # kill and rebuild it
ms edit work         # open work.toml in $EDITOR (a new name starts a new config)
ms kill work         # kill the session; the config stays
ms rm work           # delete the config; the session stays
```

A config is always named after its session. `save` captures the current
session, or `-s SESSION`. Inside tmux, `NAME` defaults to the current
session's name for `start`, `view` and `edit`; `rm` and `kill` always need it.

On an existing config, `save` rewrites only `[[windows]]`; `session`, `root`,
hooks and comments stay as written. `--force` rewrites the whole config
instead, still keeping `before` and `after`, which tmux can't capture. With
`-e`, the config is replaced only if you save in the editor and the result
parses; otherwise the capture is kept as a draft that the next `save -e`
resumes. `start --fresh` refuses to run from inside the session it
would kill. `ls` marks a config that fails to parse with `!` and its error.

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
  `#{window_layout}` string, which is what `save` captures. It defaults to
  `tiled`, and an invalid one falls back to `tiled` with a warning.
- `before` and `after` run under `sh -e` only when a new session is built,
  not when `start` attaches. A failing `before` aborts the build.
- Every root must be an existing directory; `start` checks them all before
  creating anything, so a typo leaves no half-built session.
- `session` can't contain `:` or `.`, which tmux would rewrite.

Configs live in `$XDG_CONFIG_HOME/ms`, falling back to `~/.config/ms`.
