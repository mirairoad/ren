<p align="center">
  <img src="assets/ren-512.png" width="128" alt="ren">
</p>

<h1 align="center">ren</h1>

<p align="center">A terminal IDE written in Rust that edits like any ordinary editor: no vim modes or motions, Unix-style <code>:</code> commands for files, several projects in one window, and sessions that come back after a crash. Full mouse support, built-in LSP, debugger, git panel, terminal and containers.</p>

> **ren** (錬 — to refine, to temper) is a fork of [zetsu](https://github.com/erenmentes/zetsu) by Eren Menteş (MIT),
> being reshaped 1:1 around how I write code. This repository starts a fresh history from zetsu `v0.3.0`;
> all credit for the original goes to zetsu. See [CHANGES.md](CHANGES.md) for what changed and the plan.

---

## How ren works

### No vim: you just type

There is no Normal or Insert mode, and no motions, operators, registers or counts to learn. Typing always goes into the file, and the keys do what they do in any desktop editor:

- arrows, `Home`/`End`, `PgUp`/`PgDn` move; `Ctrl+←/→` jumps a word; Shift with any of them selects;
- `Ctrl+C` / `Ctrl+X` / `Ctrl+V` copy, cut and paste (the whole line when nothing is selected), synced with the system clipboard;
- `Ctrl+Z` / `Ctrl+Y` undo and redo, `Ctrl+S` saves, `Ctrl+F` finds, `Ctrl+A` selects all, `Tab` / `Shift+Tab` indent;
- the mouse clicks, drags, double-clicks a word, triple-clicks a line, scrolls, and resizes the file tree.

The one other place you type is the `:` command line at the bottom (`Ctrl+Esc` or `Ctrl+G`, or `Esc` in terminals without the kitty keyboard protocol). Commands are words, not key chords: `:git`, `:format`, `:debug`, `:help`. A number jumps to that line. A few short names are kept for muscle memory (`:w`, `:q`, `:wq`, `:qa`).

### Files: the shell commands you already know

Files are managed with the same commands as in a shell, typed on the `:` line:

```
:touch src/parser.rs          create a file (and any missing folders)
:mkdir src/cli/args           create folders
:mv old.rs new.rs             move or rename; open tabs follow
:cp config.toml config.bak    copy (into a folder when the target is one)
:duplicate main.rs            copy next to it as main_copy.rs
:rm build.log                 remove
:cd ../other  :pwd  :open notes.md  :!cargo test
```

`Tab` completes commands and paths. Paths starting with `/` are from the workspace root (`/src/x.rs`), `~/` from your home, and anything else from the folder you're in (the tree's selection, or the open file's folder).

Nothing is deleted for real: `:rm`, and anything `:cp` or `:mv` would overwrite, goes to the project's `.bin/`. `Ctrl+Z` in the file tree undoes file commands, even after a restart (`:bin` shows what's there, `:bin empty` empties it). `:!cmd` runs any shell command, with its output in a tab.

### Several projects in one window

The first tab is not a file: it shows the open folder's name, and clicking it (or `Ctrl+R`) lists your open projects with "Open folder…" at the end. Each project has its own tabs, file tree, language servers, terminals, debugger and git panel, and keeps running while you work in another. See [Projects](#projects).

### Sessions that come back

What you have open (every project, its tabs and cursors, and unsaved text) is saved as you work. A crash, a killed process or a closed terminal loses a few seconds at most. The welcome screen lists recent sessions, and `ren --resume` lists them all with earlier checkpoints, so you can go back to one later. Every ren window has its own session. See [Sessions](#sessions).

## Features

- **Git panel** (`:git`) — staged and unstaged files with checkboxes, side-by-side diffs, staging of single lines, branches, stash, push/pull and a commit form built from your own templates.
- **Bottom panel** (`` Ctrl+` ``), under the editor (the file tree keeps its full height) — a real terminal (several sessions; hover a tab for its × to kill it), the debugger, and a Containers tab for Docker-compatible sockets (status, start/stop/restart, live logs, shell into a container).
- **File tree** — Space previews a file without leaving the tree, Enter opens it; optional Nerd Font file icons. Files with errors show red (in the tree and on their tab), files with only warnings yellow, and folders take the worst colour of what's inside. The whole project is checked when it opens, not only the files you open (see Project scan).
- **Language Server Protocol** — completion (including auto-imports and snippet expansion), diagnostics with rule codes, hover, go to definition, references, code actions with resolve support, signature help, rename, document formatting, and inlay hints (types and parameter names drawn in the code, like VS Code). Servers come from the project's `.venv` / `node_modules/.bin` first, then `PATH`, and several can run for one language (e.g. basedpyright + ruff).
- **Mouse** — click to place the cursor, drag to select, double/triple click for word/line selection, scroll, tab switching, context menu, Ctrl+click to jump to a definition, resizable file tree.
- **Debugger (DAP)** — breakpoints, stepping, variables tree, call stack, watches and a debug console. Works with CodeLLDB, lldb-dap and gdb for Rust/C/C++, debugpy for Python and Delve for Go. Missing adapters are downloaded on first use, Rust projects are built automatically, Python runs in the project's `.venv`, and `.vscode/launch.json` configurations can be picked from the Debug tab.
- **Find and replace** — inline find bar with live highlighting, match counter, case/whole-word/regex options, capture-group replacement and single-step undo for replace-all.
- **Navigation** — document symbols, workspace symbol search, references, jump list.
- **Workspace** — file tree, tabs, folder picker, recent workspaces, per-project `.ren/settings.json`, shell command integration (`:!cargo build`).
- **System clipboard** — copy, cut and paste are synced with the OS clipboard on Windows, macOS, X11 and Wayland.
- **Syntax highlighting** for Rust, C/C++, Python, JavaScript/TypeScript, Go, Shell, Lua, TOML/YAML and JSON.
- **Rendering** — double-buffered, diff-based redraw; only changed rows are written to the terminal. Truecolor with automatic 256-color fallback.

## Installation

ren is built and released for **macOS and Linux**, as ready-to-run binaries: the installer downloads one, so you don't need Rust or cargo. Windows is paused for now: its code paths and installer stay in the repo, but it doesn't build yet (the embedded terminal is Unix-only), and releases have no Windows binary.

### Linux / macOS

```sh
curl -fsSL https://raw.githubusercontent.com/mirairoad/ren/main/install.sh | sh
```

Downloads the prebuilt binary for your system from the latest release into `~/.local/bin`; nothing is compiled. Run it again to update.

- **Linux:** registers a `.desktop` entry with the application icon and places a launcher on the desktop. It runs `ghostty -e ren` when Ghostty is on `PATH`, otherwise the default terminal. The Linux binary is statically linked (musl) and has no runtime dependencies.
- **macOS:** creates `~/Applications/Ren IDE.app` with a desktop alias. The launcher opens ren in a new Ghostty window (through a login shell, so `PATH` from your shell config applies). Without Ghostty it falls back to iTerm2, then Terminal.app.

### Prebuilt binaries

Binaries for every release are available on the [Releases](https://github.com/mirairoad/ren/releases) page:

| Platform | Asset |
|---|---|
| Linux x86_64 | `ren-linux-x86_64` |
| macOS Apple Silicon | `ren-macos-arm64` |
| macOS Intel | `ren-macos-x86_64` |

Downloaded by hand, the file needs `chmod +x` (rename it to `ren` and put it on your `PATH`). On macOS, a binary downloaded in a browser is quarantined and Gatekeeper refuses to open it, since ren isn't notarized: run `xattr -d com.apple.quarantine ren` once, or use the installer above (`curl` doesn't add the quarantine flag).

### From source

Requires a stable Rust toolchain ([rustup.rs](https://rustup.rs)) and a C compiler (for `ring`, the TLS library behind the debug-adapter downloader): `gcc` or `clang` on Linux (`build-essential` on Debian/Ubuntu), and the Xcode Command Line Tools on macOS (`xcode-select --install`).

```sh
git clone https://github.com/mirairoad/ren
cd ren
cargo install --path .
```

Or build and install with the same launchers as the release installer (binary in `~/.local/bin`, Ghostty app on macOS):

```sh
./install.sh --local
```

Re-run it after pulling or editing to update. Instances that are already open keep running; the next `ren` picks up the new build.

## Usage

```sh
ren                 # start screen
ren path/to/dir     # open a workspace
ren src/main.rs     # open a file
ren --resume        # pick a saved session (ren --resume 2, or 2.1 for a checkpoint)
ren --help          # all options
```

Type `:help` on the command line for the full keymap. `Ctrl+Esc` (or `Esc` in terminals without the kitty keyboard protocol) opens the command line.

### Essential keys

| Key | Action |
|---|---|
| typing | edits the file — there is no Normal/Insert mode |
| `Ctrl+Esc` | swap between editing and the `:` command line |
| `Ctrl+G` | the `:` command line (type a line number to jump there) |
| `Esc` | close popup / selection / find bar (opens `:` when nothing is left, without kitty keys) |
| Shift + arrows, mouse drag | select (`Ctrl+A` selects all) |
| `Ctrl+←` / `Ctrl+→` | word left / right (`Ctrl+Backspace` / `Ctrl+Delete` delete a word) |
| `Tab` / `Shift+Tab` | indent / outdent the line or the selected lines |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | copy / cut / paste (the whole line when nothing is selected) |
| `Ctrl+Z` / `Ctrl+Y` | undo / redo (in the file tree: undo / redo file commands) |
| `Ctrl+S` | save |
| `Ctrl+/` | comment / uncomment the line or the selected lines (only the marker at the line start: `// a // b` → `a // b`). The marker follows the file type (`//`, `#`, `--`, `;`, `%`, …); CSS, HTML, XML and Markdown get a block comment (`/* … */`, `<!-- … -->`) |
| `Ctrl+F` / `Ctrl+H` | find / find and replace |
| `Ctrl+B` | toggle the side panel (file tree, or git with `:git`) |
| `Ctrl+W` | cycle focus: editor → side panel → bottom panel |
| `` Ctrl+` `` | bottom panel: Terminal, Debug, Containers (or `:panel`) |
| `Ctrl+PgUp` / `Ctrl+PgDn` | previous / next tab |
| `Ctrl+R` | project list: switch project, or "Open folder…" (same as clicking the first tab) |
| `:q` / `:qa` | close tab / quit |
| `:w` / `:wq` | save / save and close the tab |
| hover a tab | its icon turns into ×: click it (or middle-click the tab) to close; unsaved tabs open `:` for `q!` / `wq` |

### File tree

| Key | Action |
|---|---|
| `Space` | preview the file, focus stays in the tree (the next preview replaces it) |
| `Enter` | open the file as a normal tab and edit it |
| `Delete` / `F2` | prefill `:rm` / `:mv` for the selected entry |

### Commands

| Command | Action |
|---|---|
| `:touch <path>` (`:mkfile`) | create a file and missing folders |
| `:mkdir <path>` | create folders |
| `:cp <src> <dst>` / `:mv <src> <dst>` | copy / move (into `<dst>` when it is a folder) |
| `:duplicate <path>` | copy as `name_copy.ext` |
| `:rm <path>` | move to `.bin/` |
| `:bin`, `:bin empty` | bin status / empty it |
| `:open <path>`, `:save <path>`, `:revert` | open a file / save as / reload from disk |
| `:cd <dir>`, `:pwd` | change this project's folder / show it |
| `:!<cmd>` | run a shell command, output in a tab (`%` is the current file) |
| `:git …` | `init`, `add`, `reset`, `commit [msg]`, `push`, `pull`, `stash`, `branch`, `checkout [-b]` |
| `:terminal`, `:containers` | open those panel tabs |
| `:settings [project]` | global or project settings |
| `:project`, `:project <n>`, `:project <folder>`, `:project close` | list the open projects / switch to the n-th / open a folder as a project / close the one on screen |
| `:session`, `:session <n>`, `:session <n>.<k>`, `:session forget <n>` | list saved sessions and their checkpoints / restore one (all its projects) / restore its checkpoint k steps back / delete one (see [Sessions](#sessions)) |
| `:reload`, `:recover` | restart in place / restore the newest interrupted session with unsaved text |

Paths: `/x` is the workspace root, `~/x` your home, and anything else is relative to the folder you're in (the tree selection, or the active file's folder). Quotes only matter for spaces.

### LSP

| Key | Action |
|---|---|
| `Tab` / `Enter` | accept completion |
| `Ctrl+Space` | trigger completion |
| `F1` | hover documentation |
| mouse resting on code (1 s) | a box with the problems there (severity, tool and rule code, full message, docs link), the type / docs, and clickable quick fixes |
| `F12`, `Ctrl+Click` | go to definition |
| `Ctrl+O` | jump back |
| `Shift+F12` | references |
| `Ctrl+Shift+O` | document symbols |
| `Ctrl+T` | workspace symbols |
| `Ctrl+.` | code actions |
| `F8` / `Shift+F8` | next / previous diagnostic |
| `F2` | rename symbol |
| `:format` | format document |
| `:diag` | list diagnostics |
| `:lsp`, `:lsp restart` | server status / restart |
| `:scan` | check the whole project again now (red/yellow file names) |

### Debugging

| Key | Action |
|---|---|
| `F5` | start / continue |
| `Shift+F5` / `Ctrl+Shift+F5` | stop / restart |
| `F9` or click the gutter | toggle breakpoint |
| `F10` / `F11` / `Shift+F11` | step over / into / out |
| `F6` | pause |
| `F1` | evaluate the expression under the cursor while paused |

## Projects

The first tab is the project selector: the open folder's name with a `▾`. Click it, or press `Ctrl+R`, for the list:

- the open projects, the one on screen ticked, `●` next to one with unsaved changes;
- **Open folder…** at the end: pick a folder and it opens as another project.

Picking a project swaps everything on screen to it. Each project has its own:

- tabs, cursors and unsaved changes, file tree and focus;
- language servers and project scan (the red and yellow file names);
- terminals in the bottom panel, the debugger and its breakpoints;
- git panel and commit form, and `.bin/`;
- project settings (`.ren/settings.json`).

Projects you're not looking at keep running. Their terminals keep going, and their language servers stay up: what they report is picked up when you switch back. Containers are shared by all projects, since they belong to the Docker socket rather than to a folder.

| Command | Action |
|---|---|
| `:project` | the project list (same as clicking the first tab) |
| `:project <n>` | switch to the n-th project |
| `:project <folder>` | open a folder as a project (or switch to it if it's open) |
| `:project open` | the folder picker, to open one as a project |
| `:project close` | close the project on screen |

`:qa` with unsaved changes in another project switches to that project and says which file (`:qa!` quits anyway). `Ctrl+K`, a recent folder on the welcome screen, and `:cd` change the folder of the project on screen; use the selector to open one alongside it.

## Sessions

Every running ren saves a session: all its projects in selector order, the one on screen, each project's tabs with their cursors and scroll, the file tree (shown, width, open folders, selection), the bottom panel and focus, and copies of unsaved text. Terminal contents and debug sessions are not saved; they start fresh.

**When it's saved**

- a few seconds after anything changes (and only when something did);
- at once when the terminal closes (hang-up) or ren gets `SIGTERM`, unsaved text included;
- on a clean quit, marked clean and without the unsaved text, which quitting discarded. Text thrown away by `:qa!` still stays one checkpoint back (see below).

A crash or `kill -9` loses at most the last few seconds. A session that didn't close cleanly shows as "interrupted", with its unsaved tabs; when it left unsaved text, ren points at it when it starts.

**Several windows.** Each ren window has its own session and holds a lock on it while it runs, so another ren never lists or takes over a session that's still open. If the terminal app closes with three ren windows in it, all three are saved and listed, even when they had the same folders open.

**Getting one back**

- The welcome screen lists recent sessions above the recent folders. Press the number or click it.
- `ren --resume` (or `ren -r`) opens ren on the full list. `ren --resume 2` restores session 2 straight away.
- Inside ren, `:session` shows the same list. `:recover` restores the newest interrupted session that has unsaved text.

Restoring brings back all of the session's projects in their order, with their tabs and unsaved text (marked unsaved, not written to disk). In a ren that has just started, the session replaces the empty start; otherwise its projects join the ones already open. The ren that restores a session saves into it from then on.

**Checkpoints.** Besides its latest state, each session keeps the two before it. A new checkpoint starts when what's open (projects, and tabs other than previews) changes after staying the same for at least a minute; quicker changes, like opening several files in a row, just update the latest state. So a project or a set of tabs you closed by accident is still one step back, and so is unsaved text a quit threw away. They show as `↳` rows under their session:

```
1  beta + alpha   3 tabs · just now · ⚠ interrupted · 2 unsaved
↳  beta + alpha   5 tabs · 14 min ago · ⚠ 1 unsaved
↳  beta           2 tabs · 1 h ago
2  gamma          1 tab · 2 d ago
```

`ren --resume 1.1` or `:session 1.1` restores the checkpoint one step back. Going back keeps the newer states as checkpoints, so it can be undone.

| Command | Action |
|---|---|
| `ren --resume`, `:session` | list sessions and their checkpoints (Enter restores) |
| `ren --resume <n>`, `:session <n>` | restore session n |
| `ren --resume <n>.<k>`, `:session <n>.<k>` | restore its checkpoint k steps back |
| `:session forget <n>` | delete session n |
| `:recover`, `:recover discard` | restore the newest interrupted session with unsaved text / drop its unsaved copies |
| `:reload` | restart ren (new settings, a new build) and come back through the same session |

Sessions live in `~/.ren/sessions/`, one folder per session. ren keeps the 12 newest, one per set of projects, plus older ones that still hold unsaved text or went down together with the newest (several windows closed by one crash).

## Debugger

ren speaks the Debug Adapter Protocol. Open a file and press `F5`; no manual adapter setup is required. When no suitable adapter is found, ren downloads one into its data directory the first time and reuses it afterwards:

| Language | Adapter | Provisioning |
|---|---|---|
| Rust, C, C++ | CodeLLDB | latest release from [vadimcn/codelldb](https://github.com/vadimcn/codelldb/releases) for the current OS and CPU (~55 MB download) |
| Python | debugpy | universal wheel from PyPI, loaded via `PYTHONPATH`; neither `pip` nor a global install is needed |
| Go | Delve | built with `go install` into the data directory |

Adapters already on the system are used first: `codelldb`, `lldb-dap` or `gdb` 14+ on `PATH`, a Python environment that can import `debugpy`, or `dlv`. If a download fails (offline, blocked network), ren falls back to `lldb-dap` or `gdb` when available and reports the error in the debug console.

Python 3.8+ must be installed for Python debugging, and Go for Go debugging, since those are the runtimes being debugged.

Downloads honour `HTTPS_PROXY`/`HTTP_PROXY`/`NO_PROXY` and the operating system's certificate store.

| Command | |
|---|---|
| `:install` | show which tools are installed |
| `:install <codelldb\|debugpy\|delve>` | install or update a tool |
| `:uninstall <name>` | remove a downloaded tool |

Tools are stored in `%LOCALAPPDATA%\ren\tools` on Windows and `$XDG_DATA_HOME/ren/tools` (default `~/.local/share/ren/tools`) on Linux and macOS.

For Rust, ren runs `cargo build` and debugs the produced binary. For C/C++, pass the program with `:debug ./a.out [args]`. Program arguments can be set with `:debug args ...`.

If the workspace contains `.vscode/launch.json`, its `launch` configurations with a supported type (`lldb`, `lldb-dap`, `cppdbg`, `gdb`, `debugpy`, `python`, `go`) are listed when you press `F5` (or `:debug config`, or the `config ▾` label in the Debug tab header); the one you pick is remembered. Comments, trailing commas, `${workspaceFolder}`, `${file}`, `${env:NAME}` and similar variables are supported, as is CodeLLDB's `cargo` field. Python runs with the project's `.venv` interpreter when there is one.

CodeLLDB gives the best Rust experience: it pretty-prints `Vec`, `String`, `HashMap` and other standard types. In the debug console expressions are evaluated directly; prefix a line with `` ` `` to run a raw LLDB command (`` `frame info ``).

## Language servers

ren looks for these servers in the project first (`.venv/bin`, `venv/bin`, `node_modules/.bin`, then `$VIRTUAL_ENV`) and then on `PATH`:

| Language | Server |
|---|---|
| Rust | `rust-analyzer` |
| Python | `pyright-langserver`, `basedpyright-langserver`, `pylsp`, `ruff server` |
| JavaScript / TypeScript | `typescript-language-server` |
| C / C++ | `clangd` |
| Go | `gopls` |
| Lua | `lua-language-server` |
| Shell | `bash-language-server` |

**Inlay hints** are asked for once typing pauses, and move with your edits until the server answers again. rust-analyzer, gopls, clangd and basedpyright send them; ren turns on gopls's and TypeScript's (both off by default), and rust-analyzer's closure return types. pyright, the first Python server in the list, has none, but basedpyright does. Each kind (types, parameter names, return types, chain types, closing braces) can be switched off in `inlayHints` (see Configuration). `:hints` opens a checkbox menu to switch them for the session, and `:hints types` / `:hints off` do it from the command line.

**Rust files must be part of the crate.** rust-analyzer only works on files reached from `main.rs` / `lib.rs` through `mod name;` lines; a new file nobody declares gets no completion, hints or errors. ren checks this and warns on line 1 (`Not in the crate: add mod x; to src/main.rs`). `Ctrl+.` or `:mod` inserts the line into the parent module, unsaved.

**Project scan.** A second after a folder opens, ren checks the whole project, so the tree shows red and yellow names before you open anything:

| Project (marker at the root) | How |
|---|---|
| Rust (`Cargo.toml`) | starts rust-analyzer, which runs `cargo check` (again on every save) |
| Go (`go.mod`) | starts gopls, which checks every package |
| Python (`pyproject.toml`, `setup.py`, …) | starts pyright/basedpyright in workspace mode (every file, live); runs `ruff check` when ruff is one of the Python servers |
| TypeScript (`tsconfig.json`) | runs `tsc --noEmit` (incremental) when `tsc` is in `node_modules/.bin` or on `PATH` |

It doesn't slow ren down: the servers and checkers are separate programs and ren only reads their results. The command-line checkers (ruff, tsc) run again after a save of a file of their language (or of `pyproject.toml`, `tsconfig.json`, …), in the background, one at a time, at low priority (`nice`), and a rerun waits at least as long as the last run took, so a slow `tsc` on a big project gets at most half the CPU time. They don't run while you type: they read the saved files, and the open file's problems already come live from its server. An open file shows its server's view; the checkers' results count for files no server watches. `:scan` checks again now (e.g. after a `git pull`), and `"scan": false` turns it off for big projects (a project's `.ren/settings.json` can do it for just that one).

One server runs for every project of a language: opening a file from another project (another `Cargo.toml`, `package.json`, …) adds that folder to the running server.

By default one server runs per language: the first one found, in the order listed. To run several at once, list them in `lsp.servers` (see Configuration): diagnostics and code actions are merged, formatting goes to the `lsp.format` server, and hover/definition/completion go to the first server that supports them. Diagnostics show their source and rule code, e.g. `` `os` imported but unused [Ruff F401] ``.

The workspace root is resolved from project markers (`Cargo.toml`, `package.json`, `go.mod`, `pyproject.toml`, `compile_commands.json`, `.git`). When no server is available, completion falls back to words from open buffers.

Set `REN_LSP_LOG=<file>` to log all JSON-RPC traffic.

## Configuration

Everything lives in `~/.ren/` (`%USERPROFILE%\.ren\` on Windows). The installer creates it, and ren creates it on first start if it's missing (`ren --init` does it by hand).

`~/.ren/settings.json` holds your settings; open it with `:settings`. A project can have its own `.ren/settings.json` (`:settings project`) that merges over the global one: objects merge key by key, arrays and plain values replace. Saving either file inside ren applies it immediately. Comments and trailing commas are allowed.

```jsonc
{
  "theme": { "primary": "#DA56D1" },
  "formatOnSave": true,
  "icons": true,
  "inlayHints": { "enabled": true, "types": true, "parameters": true, "returnTypes": true, "chaining": true, "closingBraces": true },
  "lsp": {
    "servers": { "python": ["basedpyright-langserver --stdio", "ruff server"] },
    "format": { "python": "ruff" }
  },
  "containers": { "socket": "unix:///var/run/docker.sock" },
  "git": {
    "templates": {
      "default": {
        "format": ["{ticket} {type} {summary} {time}", "", "{body}"],
        "fields": [
          { "key": "ticket", "label": "Ticket", "prefix": "[", "suffix": "]" },
          { "key": "type", "label": "Commit type", "choices": [{ "value": "🩹", "label": "🩹 Bug fix" }, { "value": "🚀", "label": "🚀 New feature" }] },
          { "key": "summary", "label": "Short description", "required": true },
          { "key": "time", "label": "Time spent", "prefix": "#time ", "choices": ["15m", "1h"], "free": true },
          { "key": "body", "label": "Body", "lines": 5 }
        ]
      }
    }
  }
}
```

| Key | Default | Description |
|---|---|---|
| `theme.primary` | `"#DA56D1"` | primary colour (`#RRGGBB` or `#RGB`): logo, focused panels, badges, find and debug highlights |
| `formatOnSave` | `true` | format through the language server before writing (`Ctrl+S`, `:save`) |
| `icons` | `true` | Nerd Font file-type icons in the tree and tabs (Ghostty ships the symbols; other terminals need a Nerd Font) |
| `scan` | `true` | check the whole project when it opens and after saves, for red/yellow file names (see Project scan under Language servers). Takes effect for pyright/basedpyright on `:lsp restart` |
| `inlayHints.enabled` | `true` | types and parameter names from the language server, drawn in the code. `"inlayHints": false` is short for this |
| `inlayHints.types` | `true` | variable types: `let x: i32`, Go `range` variables, Python `x: int` |
| `inlayHints.parameters` | `true` | parameter names at call sites: `add(a: 1, b: 2)` |
| `inlayHints.returnTypes` | `true` | return types: Rust closures with a block, TypeScript `f(): number`, Python `-> int` |
| `inlayHints.chaining` | `true` | the type at the end of each line of a method chain (Rust) |
| `inlayHints.closingBraces` | `true` | what a long block closes, after its `}` (Rust, C/C++) |
| `lsp.servers.<language>` | built-in list | server command lines to run together for a language (`python`, `rust`, `typescript`, `go`, `c`, `lua`, `shell`, …) |
| `lsp.format.<language>` | first that can | name of the server that formats |
| `containers.socket` | docker's default | Docker-compatible socket for the Containers tab (OrbStack, Colima, Podman, Dory: just a different path) |
| `git.templates.<name>` | — | commit templates: `format` lines with `{key}` placeholders, and `fields` (`label`, `hint`, `prefix`/`suffix` shown only when filled, `choices`, `free`, `lines` for a multi-line box, `required`). A field called `ticket` is pre-filled from the branch name (`feat/TKT-123-x` → `TKT-123`) |

Missing keys use their defaults. Invalid values are reported in the bottom bar, and the default is used instead.

Format on save sends `textDocument/formatting` to the file's language server and waits up to 1.5 s for the edits. If there is no server for the file, it doesn't support formatting, or it is still starting, the file is written as-is. On a timeout the file is written unformatted and the bottom bar says so. One `Ctrl+Z` undoes the formatting.

`~/.ren/state` (recent workspaces) and `~/.ren/sessions/` (one folder per session: open projects and tabs, copies of unsaved text, and two earlier checkpoints in `1/` and `2/`) are managed by ren.

## Building a release

Releases are produced by GitHub Actions (`.github/workflows/build.yml`). Every push to `main` and every pull request runs `cargo fmt --check` and clippy, and builds and tests Linux x86_64 (static, musl), macOS arm64 and macOS x86_64; pushing a version tag also publishes a GitHub Release with those three binaries attached:

```sh
git tag v0.4.0
git push origin v0.4.0
```

Bump `version` in `Cargo.toml` first: the welcome screen and `ren --version` show it.

## Project layout

```
src/
  main.rs        terminal setup (kitty keyboard protocol), event loop, :reload restart
  editor.rs      modes, keys, mouse, selection, commands, file tree
  files.rs       :touch/:mkdir/:cp/:mv/:duplicate/:rm and .bin glue
  fileops.rs     file operations and the .bin undo history
  ide.rs         completion, diagnostics, code actions, menus
  lsp.rs         LSP client: several servers per language, .venv lookup, JSON-RPC
  buffer.rs      text buffer, undo/redo, word movement
  render.rs      UI layout and theme
  screen.rs      diff-based screen buffer
  icons.rs       Nerd Font file icons
  highlight.rs   syntax highlighting
  tree.rs        file tree
  find.rs        find and replace bar, search matcher
  session.rs     sessions (~/.ren/sessions): autosave, checkpoints, welcome-screen list, --resume, :reload
  projects.rs    several projects at once: the selector tab, switching, :project
  scan.rs        project scan: problems in files that aren't open
  peek.rs        the box that opens when the mouse rests on code
  comments.rs    Ctrl+/ comment markers per file type
  git.rs         git CLI: status, diffs, partial staging patches, commit messages
  git_ui.rs      git panel, side-by-side diff, commit form, branches, stash
  panel.rs       bottom panel: Terminal / Debug / Containers tabs
  term.rs        embedded terminal (pseudo-terminal + vt100)
  containers.rs  docker CLI: list, start/stop, logs, shell
  dap.rs         Debug Adapter Protocol client, adapter discovery, launch.json
  debug_ui.rs    debugger session, breakpoints and the Debug tab
  tools.rs       debug adapter download and installation
  settings.rs    global + project settings, JSONC, templates
  config.rs      ~/.ren/state (recent workspaces)
```

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, coding guidelines and the pull request process.

## License

[MIT](LICENSE)
