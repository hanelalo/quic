# quic

A terminal workspace for coding agents. Most agent CLIs live in a chat-only
window: you cannot see the project tree, and you have no idea which files the
agent just touched. `quic` puts both next to the agent.

```
┌ quic ~/src/project  ⎇ main                              ● claude  · focus: agent ┐
├───────────────────────────────┬─────────────────────────────────────────────────┤
│ FILES · 128                   │ AGENT · claude                                  │
│ ▾ src                         │                                                 │
│   ▸ ui                        │   > fix the flaky test in foo.rs                │
│   · main.rs                   │                                                 │
│   · app.rs                    │   ● Read(foo.rs)                                │
│                               │   ● Edit(foo.rs)                                │
├───────────────────────────────┤                                                 │
│ CHANGES · 2                   │                                                 │
│  M src/foo.rs                 │                                                 │
│ ?? scratch.txt                │                                                 │
└───────────────────────────────┴─────────────────────────────────────────────────┘
 j/k move · ↵ toggle · h/l collapse/expand · . hidden · r refresh · F4 switch · ^Q quit
```

- **Left (30%)**: file tree on top, live `git status` below.
- **Right (70%)**: the real agent CLI, running in a PTY — fully interactive.
- **Agents coexist**: every agent you start keeps running in the background
  with its own PTY, screen and scrollback. The right pane shows one at a time
  as tabs; `F6`/`F7` cycle, `F4` opens the switcher (where `d` stops one).
- **Choose / switch agents inside the app** with `F4`. Nothing is started
  automatically: on launch you get the agent picker (press `Esc` to browse
  files first). Built-ins: `claude` (started with
  `--dangerously-skip-permissions`), `codex`, `opencode`, `antigravity`
  (Google's `agy` CLI), and `pi` (Pi coding agent).

## Build & run

```sh
cargo build --release
./target/release/quic                 # current directory
./target/release/quic ~/src/project   # or a path
```

## Keys

| Key | Action |
| --- | --- |
| `F1` / `F2` / `F3` | Focus file tree / change list / agent |
| `F4` | Open the agent switcher (`d` stops the highlighted session) |
| `F5` | Refresh file tree and git status |
| `F6` / `F7` | Previous / next agent session |
| `Ctrl+Q` | Quit (anywhere) |
| `Ctrl+C` | Quit unless the agent pane has focus (there it interrupts the agent) |

**File tree**: `j`/`k` or arrows to move, `Enter`/`Space` to expand or collapse
a directory, `h`/`l` to collapse/expand, `g`/`G` for top/bottom, `.` to toggle
hidden files, `r` to refresh, `q` to quit.

`a` adds the selected file or folder to the current agent session: it types
`@path/to/selection/` into the agent's input (an `@`-mention, understood by
Claude Code, Codex and OpenCode) and keeps focus in the tree so you can add
several entries in a row. Paths containing spaces are quoted (`@"my file.rs"`).

**Change list**: `j`/`k` to move, `Enter` to reveal the file in the tree (expands
its parent directories), `g`/`G` for top/bottom, `r` to refresh, `q` to quit.

**Agent pane**: every key (and pasted text) is forwarded to the agent, except
the global function keys above. If the agent process exits, press `Enter` to
restart it.

## Custom agents

`--agent` only *defines* extra entries for the in-app switcher; it never picks
one at startup.

```sh
quic --agent "gpt=codex --model gpt-5.5" --agent "claude-full=claude --dangerously-skip-permissions"
```

`--agent` accepts `name=command` (or just `command`) and can be repeated. The
command runs through `/bin/sh -c "exec <command>"` with the project root as its
working directory, so arguments, env prefixes and wrappers all work. The
selected agent is started when you hit Enter in the `F4` switcher.

A spec whose name matches a built-in **overrides** that entry instead of adding
a second one, so `quic --agent claude` runs Claude Code with normal permission
prompts (no `--dangerously-skip-permissions`).

> The built-in `claude` entry runs with `--dangerously-skip-permissions`: it can
> read, edit, and execute files without asking for approval. Override the entry
> as shown above if you want the usual confirmations.

Other options:

```
--root <PATH>, -C <PATH>   project directory (default: cwd)
--refresh-ms <N>           auto-refresh interval (default: 2000)
```

## How it works

| Piece | Crate | Role |
| --- | --- | --- |
| UI + layout | `ratatui` / `crossterm` | 3:7 split, panes, key handling |
| Agent process | `portable-pty` | agent runs on a real PTY (correct `isatty`, colors, resize/SIGWINCH) |
| Agent screen | `vt100` | parses the agent's ANSI output into a cell grid we draw into the ratatui buffer |
| File tree | `ignore` | walks the project honoring `.gitignore` |
| Changes | `git status --porcelain -z` | staged/unstaged/untracked/rename entries |

Keys captured by the UI are re-encoded as xterm sequences (`src/keys.rs`) and
written to the agent's PTY. The PTY is resized together with the pane, and the
emulator answers cursor-position/device-attribute queries so full-screen agent
UIs don't stall.

The tree and change list refresh automatically every 2 seconds (configurable),
so edits made by the agent show up without you doing anything. The change list
is powered by `git status`; in a directory that isn't a git repository it shows
a notice instead.

## Development

`examples/smoke.rs` is a headless harness that runs the TUI inside a PTY, sends
keystrokes, and dumps the rendered screen as text — handy for checking layout
changes without a graphical terminal:

```sh
cargo build --example smoke
# BIN ROOT [AGENT_SPEC ...] [--type] [--reveal] [--switch] [--add]
#                          [--multi] [--stop] [--pick=N] [--settle=MS] [--quick]
./target/debug/examples/smoke target/debug/quic . "demo=cat" --type --switch
```
