<p align="center">
  <img src="assets/ren-512.png" width="128" alt="ren">
</p>

<h1 align="center">ren</h1>

<p align="center">A terminal IDE written in Rust: type-to-edit with Unix-style <code>:</code> commands, full mouse support, built-in LSP, debugger, git panel, terminal and containers.</p>

> **ren** (錬 — to refine, to temper) is a fork of [zetsu](https://github.com/erenmentes/zetsu) by Eren Menteş (MIT),
> being reshaped 1:1 around how I write code. This repository starts a fresh history from zetsu `v0.3.0`;
> all credit for the original goes to zetsu. See [CHANGES.md](CHANGES.md) for what changed and the plan.

---

## Features

- **Two modes, no vim** — you always type into the file (Edit); `Ctrl+Esc` swaps to the `:` command line (Command). Shift+arrows/mouse select, `Ctrl+C/X/V/Z/Y/S` do what they do everywhere.
- **Unix-style file commands** — `:touch`, `:mkdir`, `:cp`, `:mv`, `:duplicate`, `:rm` with paths from the workspace root (`/src/x.rs`), home (`~/`) or the current folder. Nothing is deleted: removed and overwritten files go to `.bin/`, and `Ctrl+Z` in the file tree undoes file commands, even after a restart.
- **Git panel** (`:git`) — staged and unstaged files with checkboxes, side-by-side diffs, staging of single lines, branches, stash, push/pull and a commit form built from your own templates.
- **Bottom panel** (`` Ctrl+` ``), under the editor (the file tree keeps its full height) — a real terminal (several sessions; hover a tab for its × to kill it), the debugger, and a Containers tab for Docker-compatible sockets (status, start/stop/restart, live logs, shell into a container).
- **File tree** — Space previews a file without leaving the tree, Enter opens it; optional Nerd Font file icons.
- **`:reload`** — restarts ren (new settings, new binary) and puts you back where you were, unsaved text included. Unsaved tabs are also kept safe for crash recovery (`:recover`).
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

> No ren releases are published yet, so the one-line installers below won't work until they are. Use **From source**.

### Windows

```powershell
irm https://raw.githubusercontent.com/mirairoad/ren/main/install.ps1 | iex
```

Installs to `%LOCALAPPDATA%\Programs\ren`, adds it to the user `PATH` and creates Desktop and Start Menu shortcuts. Windows Terminal is used when available.

### Linux / macOS

```sh
curl -fsSL https://raw.githubusercontent.com/mirairoad/ren/main/install.sh | sh
```

Installs the binary to `~/.local/bin`.

- **Linux:** registers a `.desktop` entry with the application icon and places a launcher on the desktop. It runs `ghostty -e ren` when Ghostty is on `PATH`, otherwise the default terminal. The Linux binary is statically linked (musl) and has no runtime dependencies.
- **macOS:** creates `~/Applications/Ren IDE.app` with a desktop alias. The launcher opens ren in a new Ghostty window (through a login shell, so `PATH` from your shell config applies). Without Ghostty it falls back to iTerm2, then Terminal.app.

### Prebuilt binaries

Binaries for every release are available on the [Releases](https://github.com/mirairoad/ren/releases) page:

| Platform | Asset |
|---|---|
| Windows x86_64 | `ren-windows-x86_64.exe` |
| Linux x86_64 | `ren-linux-x86_64` |
| macOS Apple Silicon | `ren-macos-arm64` |
| macOS Intel | `ren-macos-x86_64` |

### From source

Requires a stable Rust toolchain ([rustup.rs](https://rustup.rs)) and a C compiler (for `ring`, the TLS library behind the debug-adapter downloader): the Visual Studio Build Tools with the C++ workload on Windows (already needed by the MSVC Rust toolchain), `gcc` or `clang` on Linux (`build-essential` on Debian/Ubuntu), and the Xcode Command Line Tools on macOS (`xcode-select --install`).

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
```

Type `:help` on the command line for the full keymap. `Ctrl+Esc` (or `Esc` in terminals without the kitty keyboard protocol) opens the command line.

### Essential keys

| Key | Action |
|---|---|
| typing | edits the file — there is no Normal/Insert mode |
| `Ctrl+Esc` | swap between editing and the `:` command line |
| `Esc` | close popup / selection / find bar (opens `:` when nothing is left, without kitty keys) |
| Shift + arrows, mouse drag | select |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | copy / cut / paste (the whole line when nothing is selected) |
| `Ctrl+Z` / `Ctrl+Y` | undo / redo (in the file tree: undo / redo file commands) |
| `Ctrl+S` | save |
| `Ctrl+F` / `Ctrl+H` | find / find and replace |
| `Ctrl+B` | toggle the side panel (file tree, or git with `:git`) |
| `Ctrl+W` | cycle focus: editor → side panel → bottom panel |
| `` Ctrl+` `` | bottom panel: Terminal, Debug, Containers (or `:panel`) |
| `Ctrl+PgUp` / `Ctrl+PgDn` | previous / next tab |
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
| `:git …` | `init`, `add`, `reset`, `commit [msg]`, `push`, `pull`, `stash`, `branch`, `checkout [-b]` |
| `:terminal`, `:containers` | open those panel tabs |
| `:settings [project]` | global or project settings |
| `:reload`, `:recover` | restart in place / recover unsaved tabs after a crash |

Paths: `/x` is the workspace root, `~/x` your home, and anything else is relative to the folder you're in (the tree selection, or the active file's folder). Quotes only matter for spaces.

### LSP

| Key | Action |
|---|---|
| `Tab` / `Enter` | accept completion |
| `Ctrl+Space` | trigger completion |
| `F1` | hover documentation |
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

### Debugging

| Key | Action |
|---|---|
| `F5` | start / continue |
| `Shift+F5` / `Ctrl+Shift+F5` | stop / restart |
| `F9` or click the gutter | toggle breakpoint |
| `F10` / `F11` / `Shift+F11` | step over / into / out |
| `F6` | pause |
| `F1` | evaluate the expression under the cursor while paused |

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

`~/.ren/state` (recent workspaces) and `~/.ren/session/` (`:reload` state and crash-recovery copies of unsaved tabs) are managed by ren.

## Building a release

Releases are produced by GitHub Actions (`.github/workflows/build.yml`). Every push to `main` builds and tests all targets; pushing a version tag also publishes a GitHub Release with the binaries attached:

```sh
git tag v0.1.0
git push origin v0.1.0
```

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
  session.rs     :reload session and crash recovery (~/.ren/session)
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
