# CHANGES — ren

**ren** (錬: to refine, to temper metal) is my personal fork of
[zetsu](https://github.com/erenmentes/zetsu) by Eren Menteş (MIT).

**Goal:** refine and personalise the editor 1:1 with how I actually write code. Keep what I use,
cut what I don't, and rebuild from the ground up the parts that don't fit how I work. This
file records each change and holds the plan for what comes next.

- Forked from: zetsu `v0.3.0` @ `ffa4d51`
- Upstream remote: `upstream` → `https://github.com/erenmentes/zetsu` (for cherry-picking fixes). This repository starts a fresh git history (2026-11-12); zetsu's history stays in zetsu.
- License: MIT. Eren's copyright notice in `LICENSE` must stay.

---

## Wish list (implemented 2026-11-11)

This was the spec for the rebuild, and it is now built (see Log, 2026-11-11: "Rebuild from the wish list"). It stays here as the reference for how things are meant to work. What didn't make it, and what only got tested through the fallback paths, is in [TODO.md](TODO.md). *(suggested)* items were built unless TODO.md says otherwise; the one exception is moving `.bin` to `.ren/bin/`, which stays an open question because I never confirmed it.

### Principle: Unix, not vim
- No vim motions. No `hjkl`, no operator/motion grammar (`d2w`, `ciw`, `dt)`). Moving around uses arrows, Home/End, PgUp/PgDn and the mouse.
- `:` stays as the command line, but the commands are Unix-shaped: a verb, then paths separated by spaces. Quotes are optional and only needed for paths with spaces: `:touch "/src/file.rs"` is the same as `:touch /src/file.rs`.
- **No old vim commands.** `:w`, `:wq`, `:x`, `:e`, `:set`, `:bn`, `:bd` and the rest all go. Only `:q` (close tab) and `:qa` (exit) stay. **Ctrl+S saves.**
- **Two modes, and Ctrl+Esc swaps between them** (for now; I may revisit the key). Today ren has six vim modes: Normal, Insert, Visual, VisualLine, Command and Search. They become two:
  - **Edit**: always typing, like VS Code. Selecting uses Shift+arrows, the mouse or Ctrl+A, which replaces Visual/VisualLine. Typing `:` inserts a colon.
  - **Command**: the cursor is on the `:` line.
  - **Ctrl+Esc** in Edit opens the `:` line, and in Command goes back to Edit. In Command, plain Esc cancels and Enter runs the command; both go back to Edit.
  - Plain **Esc** in Edit only closes things: completion popup, hover, context menu, find bar.
  - Search mode (`/`) goes; the existing find bar (Ctrl+F) covers it.
- Ctrl+Esc works the same in the tree and git panels. Esc no longer moves focus from the tree back to the editor; Ctrl+W (cycle focus) and Ctrl+B (toggle tree) already handle that.
- **Catch: the terminal has to report Ctrl+Esc.** In the standard key encoding, Ctrl+Esc arrives as plain Esc, just as Ctrl+` arrives as Ctrl+Space (see 5). ren needs the kitty keyboard protocol to tell them apart. Ghostty supports it, and crossterm can switch it on (`PushKeyboardEnhancementFlags`) and detect support (`supports_keyboard_enhancement()`). ren doesn't use it yet, so it's a prerequisite for the rebuild.
  - *(suggested)* Fallback for terminals without it (e.g. Terminal.app): plain Esc opens the `:` line once there's nothing left to close.
  - On Windows, Ctrl+Esc opens the Start menu, so a Windows build would need another key.

### 1. File commands
| Command | Does |
|---|---|
| `:touch <path>` | Create an empty file, plus any missing parent folders. If the file exists, do what Unix `touch` does: no error, just update its modified time. *(suggested)* Say "already exists" in the status line so it isn't silent. `:mkfile` is an alias. |
| `:mkdir <path>` | Create a folder, with parents (always `-p`). |
| `:cp <src> <dst>` | Copy. If `<dst>` is a folder, copy into it. Folders copy recursively. |
| `:duplicate <path>` | Copy next to the original as `name_copy.ext` (`name_copy2.ext` if that's taken). |
| `:rm <path>` | Move to `.bin` (see 2). Never a hard delete. Works on folders. |
| `:mv <src> <dst>` *(suggested)* | Move/rename. Undoable like the rest. |

Path rules:
- `file.rs` (relative) lands in the **parent folder of where I am**: the selected node's folder when the tree has focus, otherwise the active file's folder. Same for `:cp` destinations (`./` = that parent).
- `/src/file.rs` (leading `/`) means the **workspace root**, not the filesystem root.
- To get outside the workspace: `~/…` for my home folder, or walk out with `../`.
- *(suggested)* Tab completes paths on the `:` line.

### 2. `.bin`: file-level undo
- `:rm` moves the file/folder into `<workspace>/.bin/` instead of deleting it. *(suggested)* Anything `:cp`/`:mv` would overwrite goes there too.
- The first time ren creates `.bin` in a git repo, it asks to add `.bin` to `.gitignore`.
- **Ctrl+Z / Ctrl+Y** undo/redo file operations (restore from `.bin`, undo a `touch`/`mkdir`/`cp`/`duplicate`). *(suggested)* Scope: Ctrl+Z undoes file ops while the tree has focus, and text edits while a file has focus.
- **History persists between sessions.** It's cheap: `.bin` is inside the workspace, so a snapshot is a rename on the same disk (no copy), and the history is a small manifest (`.bin/index.json`: what, original path, when). *(suggested)* Cap it at 100 ops, drop the oldest, and add `:bin empty` to clear it.

### 3. File tree: preview vs open (like VS Code)
- Arrows move the selection. No `j/k/g/G`.
- **Space** previews the selected file in the editor area, and **focus stays in the tree**. Pressing Space on another file replaces the preview. There's one preview slot, like VS Code's italic preview tab.
- **Enter** opens the file as a real (anchored) tab and moves focus into it. On a folder it expands/collapses.
- *(suggested)* Editing a preview anchors it, as VS Code does.

### 4. File-type icons (on/off)
- Nerd Font glyphs per file type and for folders in the tree (and maybe in tabs). Toggle with `"icons": true|false` in settings.
- Ghostty has Nerd Font symbols built in, so they work there without installing a font. Other terminals need a Nerd Font, which is why it's a toggle. Off = today's plain names.

### 5. Bottom panel (Ctrl+`)
Ctrl+` toggles a panel at the bottom with tab buttons:
- **Terminal**: an embedded shell, started in the workspace root. This is the biggest item on the list (needs a PTY plus a terminal emulator/VT parser).
- **Debug**: the existing DAP UI (`dap.rs`, `debug_ui.rs`) moves into this tab and replaces the current debug layout.
- **Containers**: two columns.
  - **Left:** the container list (name, image, ports), with the **status shown by colour and icon**: running green, paused/restarting yellow, exited grey, dead/unhealthy red.
  - **Right:** clicking a container shows its **logs**. *(suggested)* They follow live, like `docker logs -f`.
  - **Actions:** **start** and **stop**. *(suggested)* Restart.
  - **Shell into it:** opens a shell inside the container in the Terminal tab (`docker exec -it <id>`, `bash` with a fallback to `sh`). *(suggested)* The Terminal tab can hold several sessions, so this opens a new one next to my normal shell.
  - The socket is set in settings. OrbStack, Colima and Podman are just a different socket path.
    ```json
    { "containers": { "socket": "unix:///var/run/docker.sock" } }
    ```
  - *(suggested)* **How:** list, start, stop and logs go straight to the Docker HTTP API over the socket. The shell runs the `docker` CLI with `DOCKER_HOST` set to that socket, because an interactive TTY is far simpler there.
- Technical catch: most terminals send Ctrl+` as the same byte as Ctrl+Space. Telling them apart needs the kitty keyboard protocol, the same prerequisite as Ctrl+Esc (see Principle). Without it, Ctrl+Space would also toggle the panel.

### 6. `:settings` and `:reload`
- `:settings` opens `~/.ren/settings.json`. **Done** (see Log, 2026-11-11).
- **Project settings, VS Code style.** A project can have its own `.ren/settings.json` that merges over the global `~/.ren/settings.json`.
  - Objects merge key by key. Arrays and plain values in the project file replace the global ones.
  - So a project can override a single key, e.g. `formatOnSave`, and inherit everything else.
  - Named commit templates (see 7) merge by name: a project can add its own template or replace `default`.
  - The project file is meant to be committed, like `.vscode/`.
  - Saving either file re-applies the settings immediately, as the global one does today.
  - *(suggested)* `:settings` opens the global file and `:settings project` opens (or creates) the project's.
  - *(suggested)* `.bin` moves to `.ren/bin/`, so ren keeps one folder per project. Only `.ren/bin/` would then be gitignored, not the whole `.ren/`.
- `:reload` restarts ren and puts me back where I was. It saves the session (workspace, open tabs and cursor positions, tree expansion and selection, panel state), re-execs the binary and restores the session. This also picks up a freshly installed binary after `install.sh --local`.
- **Unsaved changes survive the reload.** Before restarting, ren writes each unsaved buffer to a temp area **outside the project**, in `~/.ren/` (e.g. `~/.ren/session/<workspace>/`). It reopens them as still-unsaved tabs, then deletes the temp copies. Nothing is lost, and nothing is written to the real files until I save.
- *(suggested)* The same temp copies give crash recovery (VS Code's "hot exit"). If ren dies, the next start offers to restore the unsaved tabs.

### 7. `:git` panel (like Zed's git panel)
`:git` swaps the side panel from the file tree to a git view. *(suggested)* `:git` again, or Esc, swaps back.
- **Two sections:** **Staged** on top, **Changes** (unstaged and untracked) below. Each file has a checkbox `[ ]`/`[x]`. Ticking it stages the file, which moves it to Staged; unticking unstages it. *(suggested)* Each section header has a checkbox to tick or untick everything in it.
- **Keys:** arrows move the selection and preview that file's diff. **Space ticks/unticks** (stage/unstage). Enter opens the real file as an anchored tab. Clicking a checkbox ticks it; clicking a file name previews its diff.
- **Diff preview:** shown in the preview slot (the same single slot as the tree preview in 3). VS Code style, **side-by-side**: old on the left, new on the right, removed lines red, added lines green, syntax highlighted, read-only.
- **Partial staging:** select lines in the diff and stage just those. *(suggested)* With no selection, Space in the diff stages the hunk under the cursor.
- **Branch:** the current branch name sits at the top of the panel. Clicking it opens the branch list (same as `:git checkout`).
- **Actions:**
  - **Commit:** opens a form built from the commit template (below), then commits what's staged.
  - **Push:** sets the upstream on the first push.
  - **Stash:** stash, plus a stash list with pop, apply and drop.
  - *(suggested)* Pull.
- **Commit templates are ren's own format.** No compatibility with VS Code or its extensions: their settings are a trap. The idea is the one from the commit-template extension I use (fields I fill in, plus a pattern that builds the message), but the keys and rules are ren's. Templates live under `git.templates`, globally in `~/.ren/settings.json` or per project in `.ren/settings.json` (see 6). Here is my current template rewritten in ren's format:
  ```json
  "git": {
    "templates": {
      "default": {
        "format": ["{ticket} {type} {summary} {time}", "", "{body}"],
        "fields": [
          { "key": "ticket",  "label": "Ticket", "hint": "TKT-000123, or several", "prefix": "[", "suffix": "]" },
          { "key": "type",    "label": "Commit type",
            "choices": [{ "value": "🩹", "label": "🩹 Bug fix" }, { "value": "🚀", "label": "🚀 New feature" }] },
          { "key": "time",    "label": "Time spent", "prefix": "#time ",
            "choices": ["5m", "15m", "30m", "1h", "2h", "4h", "8h"], "free": true },
          { "key": "summary", "label": "Short description", "required": true },
          { "key": "body",    "label": "Body", "lines": 5 }
        ]
      }
    }
  }
  ```
  It produces `[TKT-000123] 🩹 fix login redirect #time 15m`, then a blank line, then the body.
  - `format` is a list of lines with `{key}` placeholders.
  - **Field kinds:**
    - one-line text by default.
    - `choices` makes it a pick list. `"free": true` also allows typing a value that isn't in the list.
    - `lines` makes it a multi-line box that many lines tall.
  - **Choices** are plain strings, or `{ "value", "label" }` when the list should show more than what goes into the message.
  - `hint` is grey help text on the field, and `required` blocks the commit while the field is empty.
  - `prefix`/`suffix` only appear when the field has a value, so an empty ticket leaves no `[]` and an empty time leaves no `#time `.
  - *(suggested)* After filling in, collapse doubled spaces and trim line ends, so a missing `{ticket}` doesn't leave a leading space. Drop trailing blank lines too, so no body means no dangling blank line.
  - Several named templates can sit side by side under `git.templates`. *(suggested)* I pick one per commit, and the last one used is the default.
  - *(suggested)* A field can be pre-filled from the branch name, e.g. `ticket` from `feat/TKT-000123-login`.
- **Command line first.** Everything in the panel is also a `:git` subcommand that mirrors the real git CLI, so I can drive git mostly from `:`:

  | Command | Does |
  |---|---|
  | `:git` | Toggle the git panel. |
  | `:git init` | `git init`, plus a `.gitignore` with `.bin` and the standard entries (below). |
  | `:git add <path>` / `:git reset <path>` *(suggested)* | Stage / unstage. |
  | `:git commit` | Open the template form. *(suggested)* `:git commit "msg"` skips the form for a quick commit. |
  | `:git push` / `:git pull` | Push (sets the upstream the first time) / pull. |
  | `:git stash` | Stash. Also `:git stash list`, `pop`, `apply` and `drop`. |
  | `:git branch <name>` | Create a branch. Without a name, opens the branch list. |
  | `:git checkout <name>` | Switch branch. `-b <name>` creates and switches. **Without a name, opens the branch list** to pick from. |

- **`.gitignore` from `:git init`**: always `.bin/`, plus OS/editor junk (`.DS_Store`, `Thumbs.db`, `*.swp`, `.idea/`, `.vscode/`), `.env` and `.env.*`, and `*.log`. *(suggested)* Language entries based on what's in the folder: `Cargo.toml` → `/target`; `package.json` → `node_modules/`, `dist/`; Python → `__pycache__/`, `.venv/`. If a `.gitignore` already exists, only the missing lines are appended.
- *(suggested)* `checkout` refuses while there are unsaved buffers, and reloads open files from disk after switching.
- *(suggested)* **How:** shell out to the `git` CLI (`git status --porcelain=v2`, `git diff`, …) rather than linking libgit2. That means no new C dependency, and my git config, credential helper/ssh agent, hooks and commit signing all just work, push included.

### 8. Debug tab driven by `.vscode/launch.json`
ren reads only `.vscode/launch.json` from `.vscode/`, never `settings.json` or anything else, and never writes there. The file is JSONC (comments and trailing commas allowed). `strip_jsonc` in `src/dap.rs` already handles both.
- **List and play.** The Debug tab (see 5) lists every configuration in `launch.json` by its `name`. Pick one and press ▶ (or F5) to launch it. *(suggested)* The last one used stays selected. With no `launch.json`, the list shows ren's current auto-detected plan (e.g. a cargo build for Rust) as a single entry.
- **Then monitor**, with what ren's debugger already has: breakpoints (F9, set before or during the run), stepping (F10/F11/Shift+F11), pause/stop/restart, variables tree, call stack, watches and the debug console. Breakpoints work the same whichever configuration launched the run.
- *(suggested)* Breakpoints and watches persist per project in `~/.ren/session/<workspace>/`, not in the repo. VS Code keeps its breakpoints in its own private storage, so there's nothing in `.vscode/` to share with it.
- **What ren does today** (`from_launch_json`, `src/dap.rs`): it silently takes the **first** `launch` config whose `type` it knows (`lldb`/`codelldb`, `lldb-dap`, `cppdbg`, `gdb`, `debugpy`/`python`, `go`). It substitutes `${workspaceFolder}`, `${file}`, `${fileDirname}`, `${fileBasename}` and similar. Gaps to close:
  - **The picker** (above): today there's no list, just the first match.
  - `attach` configurations are skipped.
  - `preLaunchTask` is ignored (no `tasks.json` support).
  - `${env:…}` and `${input:…}` aren't substituted.
  - There are no JS/TS/Deno debug types (`node`, `pwa-node`), so my TS projects can't be debugged yet.

### 9. Language servers: project `.venv` first, several servers per language
Based on the Python project test (see Log, 2026-11-11).
- **Isolation: the project's `.venv` comes first.** Each project uses its own pinned ruff, basedpyright and Python, the same versions `uv run` and CI use. Lookup order for language servers and for the Python that runs the program being debugged:
  1. `<root>/.venv/bin`
  2. `$VIRTUAL_ENV/bin`
  3. `PATH` (fallback)

  The Python project test failed in both places because this was missing: no servers found, and debugging without `launch.json` hit `ModuleNotFoundError`.
- **Several servers per language, chosen in settings.** People use different tooling, so the servers are configured in `lsp.servers`, per language. The project's `.ren/settings.json` overrides the global `~/.ren/settings.json` (see 6). With no setting, ren uses its built-in defaults (today's candidate list).
  ```json
  "lsp": {
    "servers": { "python": ["basedpyright-langserver --stdio", "ruff server"] },
    "format":  { "python": "ruff" }
  }
  ```
  - Every listed server runs at once. Their diagnostics merge.
  - Hover, definition, completion and rename go to the first server that supports them.
  - Formatting goes to the `format` server, or else the first server that can format.
  - Code actions are collected from all servers.
  - For Python that means basedpyright for types/hover/definition and ruff for lint, fixes, organize imports and formatting.
- ~~Add `ruff server` to the Python candidates.~~ **Done** (see Log). It's last in the list, so it's only picked when no pyright/pylsp is found, until several servers can run.
- ~~Show the rule code in diagnostics.~~ **Done** (see Log).

### 10. Better syntax colours: inside strings
- **Escapes stand out:** `\n`, `\t`, `\"`, `\u{1F600}` get their own colour inside a string, as in `println!("Hello, {name}!,\nI am {age}");`.
- **Placeholders are white:** `{name}`, `{age}`, and `{}` with nothing inside, read as code (normal text colour), not as string text.
- ~~**The quotes themselves are white** too.~~ **Changed:** the quotes `"` stay green, like the string text (prefixes like `r#"`, `f"`, `b"` too).
- ~~All of the above.~~ **Done** (see Log, "Syntax colours inside strings").

### Open questions (wish list)
1. ~~**Modes.**~~ **Decided (for now):** Edit + Command, Ctrl+Esc swaps between them (see Principle).
2. ~~Vim commands.~~ **Decided:** none survive except `:q`/`:qa`. Ctrl+S saves.
3. ~~`:touch` on an existing file.~~ **Decided:** Unix behaviour (update the timestamp, no error).
4. ~~Real absolute paths.~~ **Decided:** `~/…`, or walk out with `../`.
5. ~~`.bin` history.~~ **Decided:** persisted between sessions.
6. ~~`:reload` with unsaved changes.~~ **Decided:** keep them, via temp copies in `~/.ren/`.
7. ~~Containers tab.~~ **Decided:** status colour and icon, start/stop, logs column, shell into the container.
8. ~~Git panel keyboard.~~ **Decided:** Space ticks; arrows preview the diff.
9. ~~Diff layout.~~ **Decided:** side-by-side, VS Code style.
10. ~~Partial staging.~~ **Decided:** yes, stage the selected lines.
11. ~~Git scope.~~ **Decided:** stage, commit, push, pull, stash, `init`, branch create and checkout list. Log/history and blame aren't on the list.

---

## Log

### 2026-11-16: .env highlighting
- **`.env` files** are highlighted (status bar: `Dotenv`): `.env`, `.env.template`, `.env.local`, `.env.example`, `.env.production.local` and any other `.env.<name>`, plus `*.env`. A variant ending in a language's own extension (`.env.json`, `.env.sh`) stays that language; `.envrc` is a shell script, not a dotenv file.
  - Keys in the type colour, `export` as a keyword, `#` comments (whole lines, and after a value when a space comes before the `#`, so `https://x/#frag` stays part of the value).
  - Values: `"…"` with escapes and `$VAR`/`${VAR}` marked, `'…'` and `` `…` `` as written; all three can run over several lines (certificates, keys). A bare value is a string, with `$VAR` marked, or a number (`3000`, not `127.0.0.1`) or `true`/`false` when that's all it is. `KEY: value` works too.
  - Code fences marked ```` ```env ```` or ```` ```dotenv ```` in Markdown use it. The key icon now follows the same names (`prod.env` gets it, `.env.json` gets the JSON one).

### 2026-11-16: Markdown: highlighting and a rendered view
- **Markdown highlighting**: headings (bold), `**strong**`, `*emphasis*`, `***both***`, inline code, links (text and target), autolinks and bare URLs, list markers, task boxes, quote markers, table pipes, thematic breaks, HTML tags and comments (also over several lines). Fenced code blocks are highlighted in their own language (```` ```rust ````, `py`, `sh`, `ts`, `sql`, `cs`, `xml`/`html`, `dockerfile`, …), with that language's state carried from line to line inside the fence; an unknown language gets the code colour. `snake_case` isn't emphasis.
- **Rendered view** (`markdown.rs`): `Ctrl+Shift+V` (as in VS Code), `:preview`, or the status bar's `◧ Preview` switch shows the file rendered, `Ctrl+Shift+V` / `◧ Raw` / `Esc` goes back to the text at the same place (the rendered rows remember their source line both ways).
  - Paragraphs joined and wrapped to the window (at most 100 columns), hard breaks (two spaces, `\`, `<br>`).
  - Headings in colour, H1 and H2 underlined; setext headings; thematic breaks as a rule.
  - Lists with `•` `◦` `▪` by depth, numbers kept, `☐`/`☑` task boxes, hanging indents, code blocks inside items indented under them.
  - Quotes with a bar, GitHub alerts (`> [!NOTE]`, `TIP`, `IMPORTANT`, `WARNING`, `CAUTION`) with a coloured label.
  - Tables drawn as a grid, columns aligned as the separator row says; too wide, the widest column gives way and its cells wrap.
  - Code blocks as a tinted card with the language's colours and its name in the corner.
  - Inline: bold, italic, both, strikethrough (dimmed), code on a tint, link text in link colour (the target hidden), images as `[image: alt]`, entities, escapes.
  - README HTML: `<p align="center">` / `<div align="center">` centre what's inside, `<h1>`..`<h6>` are headings, `<img alt>` an image line, `<code>`/`<b>`/`<em>`/`<br>` inline; other tags and HTML comments go.
  - Read-only: arrows, PgUp/PgDn, Space, Home/End and the wheel scroll; typing says how to get back; `Ctrl+F` switches back to the text first. The status bar says `PREVIEW`.
- `Ctrl+Shift+V` only reaches ren in terminals that pass it on with the kitty keyboard protocol (Ghostty on macOS, kitty); most Linux and WSL terminals keep it for their own paste, hence `:preview` and the clickable switch.
- Tested in tmux on ren's README (centred title and image line, the quote, headings, bullets, the code card, the keys table with wrapped cells, back to line 126 from the table) and a test file with an alert, task boxes, nested lists, a setext heading, a code block in a list item, an aligned table, strikethrough, `***both***`, a hard break and an autolink. Unit tests for the highlighter (lines, fences in their language, HTML comments) and the renderer (inline markup, wrapping, blocks, tables, README HTML); 69 pass.

### 2026-11-16: SQL, Dockerfile, C#, XML, .sln and notebooks
- **Highlighting for six more kinds of file**, by file name, so any such file in the project gets it:
  - **SQL** (`.sql`, `.pgsql`, `.ddl`): keywords and types in any case (SQL is often all capitals, so capitals don't mean types here), `--` and `/* */` comments, `'…'` strings over several lines, `"Quoted"` and `` `quoted` `` names left alone, parameters `@p`, `@@version`, `$1`, `:name` (not a `::type` cast).
  - **Dockerfiles**: `Dockerfile`, `Dockerfile.flyway`, `Dockerfile.dev`, `api.dockerfile`, `Containerfile` (not `Dockerfile.dockerignore`). Instructions only count as the line's first word (after `ONBUILD`), so `from` in a `RUN` line stays a word; `AS` in `FROM … AS stage`; `$VAR` and `${VAR}`; shell strings.
  - **C#** (`.cs`, `.csx`): `$"…{x}…"` interpolation with `{{ }}`, verbatim `@"…"`, raw `"""…"""` over several lines, `#region`/`#if` directives.
  - **XML**, a new kind of highlighter (markup, not code): tag names, attribute names, values, entities (`&amp;`), comments and tags over several lines, `<?xml ?>`. MSBuild's `$(Prop)`, `@(Items)`, `%(Meta)` are marked. For `.xml`, `.csproj`/`.fsproj`/`.vbproj`, `.props`, `.targets`, `.slnx`, `.xaml`, `.axaml`, `.resx`, `.nuspec`, `.config`, `.xsd`, `.xsl`, `.svg`, `.plist`.
  - **Visual Studio solutions** (`.sln`): `Project`/`EndProject`/`GlobalSection`… as keywords, strings with backslash paths as they are, `#` comments.
  - **Jupyter notebooks** (`.ipynb`) as JSON.
- Icons for C#, solutions and project files, XML, notebooks and every Dockerfile name; `Ctrl+/` markers for `.sln` (`#`), `.slnx`/`.resx`/`.nuspec`/`.config` (`<!-- -->`) and every Dockerfile name.
- **Language servers**: C# (`csharp-ls`, then `OmniSharp -lsp`), Dockerfile (`docker-langserver --stdio`), SQL (`sqls`, then `sql-language-server`). C#'s root is the folder with a `.sln`, `.slnx` or `.csproj`: root markers can now be `*.ext`.
- **Pull diagnostics**: csharp-ls gives diagnostics only when asked (`textDocument/diagnostic`), so ren asks for the open file's once typing pauses, and again on `workspace/diagnostic/refresh`. Only servers that offer it and haven't sent any diagnostics by themselves are asked, so rust-analyzer, pyright and the rest work as before.
- **Project scan for C#**: with a `.sln`/`.slnx` (else one `.csproj`) at the root and `dotnet` on `PATH`, `dotnet build` runs a second after the folder opens and after saves of `.cs`, project and solution files, like ruff and tsc; its `File.cs(3,7): error CS0103: …` lines are tsc's shape. Several solutions (or several projects and no solution) at the root: no build, as `dotnet build` wouldn't pick either.
- Tested in tmux on a `dotnet new console` project: the tree showed `Program.cs` red (an error) and `Other.cs` yellow (warnings) from `dotnet build`; every new file type opened with the expected colours; csharp-ls 0.15 (in a temporary tool folder) found the solution through the `*.sln` marker, sent inlay hints, and its pulled diagnostics showed in `:diag` and the gutter, and updated after an edit. Unit tests for each highlighter, file-name detection, the build output and target choice, icons and comment markers; 62 pass.

### 2026-11-16: First release, 0.4.0
- **`v0.4.0` is ren's first release**, with prebuilt binaries for Linux x86_64, macOS arm64 and macOS x86_64, so `install.sh` downloads one and nobody needs cargo. The version moves on from zetsu's 0.3.0, which the fork started from.
- README: the "no releases yet" note is gone; the installer section says it downloads a binary; downloading by hand needs `chmod +x`, and on macOS `xattr -d com.apple.quarantine` for a browser download (ren isn't notarized; `curl` doesn't quarantine).

### 2026-11-16: CI and releases: macOS and Linux only
- **The build matrix is Linux x86_64 (musl), macOS arm64 and macOS x86_64**, and a release attaches those three binaries. Windows is paused, not removed: its code paths and `install.ps1` stay, and the matrix entry is kept as a comment in `build.yml`. It doesn't compile at the moment anyway (the embedded terminal, `term.rs`, is Unix-only), which is why every CI run so far had failed.
- **Two clippy errors that only CI saw** (its stable Rust is newer than the local 1.97): a range loop in `Buffer::get_text` that only indexed `lines`, now a slice loop; and `openpty` taking `&mut ws` where Linux's libc wants `*const winsize` (macOS's wants `*mut`), now a raw pointer that fits both.
- README: the install section says macOS and Linux, with Windows paused; the prebuilt table and the source requirements drop Windows; "Building a release" lists the three targets.

### 2026-11-16: Session checkpoints, `ren --resume`
- **Each session keeps its last 3 states**: the latest (`session.json`, `unsaved/`) and two checkpoints before it in `1/` and `2/`. A new checkpoint starts when what's open (projects, and tabs other than previews; order and cursors don't count) changes after staying the same for a minute. Quicker changes just update the latest state, so the three cover layouts that lasted, not every file opened in passing. A clean quit that throws unsaved text away (`:qa!`) also starts one, so that text stays one step back.
- The latest state is copied, not moved, into `1/`, so a crash between the copy and the next write still finds it. A session whose `session.json` is lost is still listed from its checkpoints.
- Closing every tab no longer deletes a session that had something worth keeping (it lasted a minute, or has checkpoints): the empty state is saved and the last real one becomes a checkpoint. A ren opened and closed in passing still leaves nothing.
- **`ren --resume`** (`-r`) opens ren on the session list; `ren --resume 2` restores session 2, `ren --resume 2.1` its checkpoint one step back. `:session` shows the same list: each session, then `↳` rows for its checkpoints. `:session <n>.<k>` restores from the command line. The welcome screen points at `:session` when there are checkpoints.
- Restoring a checkpoint takes the session over as before, and pushes the newer states back as checkpoints, so going back can be undone.
- **Windows that go down together are all kept.** Only one session per set of projects is listed, but an interrupted one saved within a minute of the newest for the same folders now stays too (several windows closed by one terminal crash). Tested: two ren windows on the same folder, `tmux kill-session`, both listed.
- **README**: a "How ren works" section up front (typing without vim modes or motions, the shell-style file commands, projects, sessions), and full Projects and Sessions sections (what's per project, what's shared, when sessions save, several windows, checkpoints, `ren --resume`, commands).
- `:open /abs/path` opens the real path when it exists and the workspace has no such path (a leading `/` still means the workspace root first). Found while testing.
- Tested in tmux (sandbox HOME): an edit, a minute, then a second file → `1/` holds the one-tab state with the unsaved copy. After `kill -9`, `ren --resume` listed the session with a `↳` row; Enter on it restored the old state with its unsaved text, and the newer state moved to `1/`. `:qa!` with unsaved text left the text in `1/`. `:session 2.1` and `ren --resume 1.1` (no such checkpoint: clear error). Unit tests for the rotation, the shape and `n.k` parsing; 55 pass.

### 2026-11-13: Sessions
- **What's open is saved as you work**, in `~/.ren/sessions/<id>/`, one folder per running ren: every project (in selector order, with the one on screen), each with its tabs, cursors and scroll, the tree (shown, width, open folders, selection), the bottom panel and focus, plus copies of unsaved text. It's written a few seconds after something changes (only when it did), into a side file that's then renamed, so a crash mid-write leaves the previous one.
- **A crash, a killed process or a closed terminal loses at most those few seconds.** Closing the terminal window sends ren SIGHUP; ren now catches it (and SIGTERM), saves at once and leaves. A write error to the terminal does the same, instead of the clean-quit path that used to delete the unsaved copies.
  - crossterm keeps reading a hung-up terminal forever (the read returns nothing and never "would block"), so catching SIGHUP alone left ren spinning at full CPU. The handler also puts an empty non-blocking pipe in place of stdin (`dup2`, safe in a signal handler); crossterm's read then gives up and the loop gets to save and leave.
- **The welcome screen lists recent sessions** above the recent folders: the projects' names (`ren + alpha`), tab count, when, and in yellow `interrupted` (it didn't close cleanly) and how many tabs were unsaved. The numbers go on from sessions into folders, so `1`–`9` reach both; a click works too. Restoring opens every project of the session with its tabs and unsaved text; when ren was started empty, the session replaces the empty start, otherwise its projects join the open ones (an open folder keeps its tabs). The restored ren takes the session over and keeps saving into it.
- One entry per set of projects (the newest; an older one stays if it holds unsaved text), at most 12 kept, the rest deleted. A run with nothing worth restoring (one project, no file tab, no typed text) leaves no session.
- **Each running ren locks its session folder** (`File::try_lock`), so a second ren doesn't list a session that's still open, and can't take it over.
- A clean quit keeps the session, marked clean, without unsaved copies: `:qa!` discarded them on purpose.
- `:session` lists the sessions, `:session <n>` restores one, `:session forget <n>` deletes it. `:recover` restores the newest interrupted session with unsaved text (`:recover discard` drops its copies), and starting ren after a crash says which one to restore.
- `:reload` goes through the same session (`ren --restore <id>`), so every project comes back in its order. A `:reload` from an older ren (which kept one session per folder in `~/.ren/session/`) still restores, and moves it into the new folder.
- Tested in tmux:
  - Two projects with unsaved tabs, then the tmux session killed (SIGHUP). ren exited, and an edit typed 0.3 s before the kill was in the session, marked interrupted.
  - A fresh `ren` said which session to restore and listed it on the welcome screen. `1` brought back both projects, in order, with their tabs and unsaved text, and replaced the empty start.
  - A second ren didn't list the session while the first was still running.
  - `:qa!` left a clean session, listed without the warning.
  - `:reload` kept the order and the unsaved text.
  - `kill -9` 4 s after an edit: the autosaved edit was there.
  - A `:reload` from the previous build into this one restored both projects and the unsaved text.

### 2026-11-13: Projects
- **The first spot of the tab bar is a project selector.** It shows the open folder's name as it is (with a folder icon when icons are on, and `▾`), and is never a file tab. Clicking it (or `Ctrl+R`, after VS Code's "Open Recent") opens a list: every open project, the one on screen ticked and `●` on those with unsaved changes, then **Open folder…** as the last item. Open folder… browses from the folder around the current project; "Open as a project" opens the folder as a new project and switches to it. A folder that's already open just comes on screen.
- **The file tree lost its title row** (the folder's name in capitals): the selector now names the folder, so the tree starts one row higher and shows one more entry. Which panel has focus still shows in the status bar badge and the selection's highlight.
- **Each project is its own workspace** (`projects.rs`): tabs and cursors, the file tree, language servers and their diagnostics (so the red/yellow names too), the project scan, the jump list, the find bar, the debugger (breakpoints, session, console, launch config), the bottom panel's state, terminals, the git panel and commit form, the `.bin` history, and its `.ren/settings.json`. **Containers are shared**: they belong to the Docker socket, not to a folder (a project whose settings name another socket switches the Containers tab to it, as before).
- **Switching doesn't stop anything.** The project leaving the screen is parked as it is: its language servers keep running (their messages wait until it's back), its terminals keep reading output in the background, and a debug session stays where it is. The one coming back gets its folder as the working directory, reloads its settings, and reloads tabs whose files changed on disk meanwhile. Unsaved text of the project leaving is copied to its session folder at once (the 3-second autosave only looks after the project on screen), so crash recovery covers every project.
- How: the editor keeps one project's state in its own fields, as before; the others sit in a `Project` struct with the same fields, and switching swaps them. Nothing else in ren had to learn about projects.
- `:project` lists them, `:project <n>` switches, `:project <folder>` opens one (an absolute path, or the usual `/x` / `~/x` / relative forms), `:project open` browses, `:project close` closes the one on screen (its servers stop, its terminals and debug session end, as on quit) unless it has unsaved changes.
- **Quitting checks every project**: `:qa` and `:q` on the last tab bring up a parked project with unsaved changes and stop there (`:qa!` still quits). Language servers of all projects shut down on quit.
- **`:reload` brings every project back**, in the same order, each with its tabs, tree, panel and unsaved text; the list is kept next to the session of the project on screen.
- Tested in tmux: opening a second project from the list (keyboard and mouse), a terminal per project (each kept its own shell and output across switches), tabs and focus per project, `:qa` stopping on the other project's unsaved file, `:reload` with two projects (both back, unsaved text kept), `:project close`, opening by path and switching to an already-open folder, and rust-analyzer staying alive with its error while another project was on screen, then shutting down on quit.

### 2026-11-13: Lower tab bar
- **The dark tab bar is a row and a half instead of two rows.** The padding row above the labels now has only its lower half dark (`▄` in the bar colour over the editor background), so the band is half a row lower while the tabs still stand taller than a single row. The open tab keeps its primary `▁` line right on its label, now over the editor background. Layout and mouse targets are unchanged (the tabs still take two rows of the screen).

### 2026-11-13: Project scan
- **A second after a folder opens, ren checks the whole project** (`scan.rs`), so red/yellow file names show before anything is opened. Before, a language server only started with the first file of its language, and most only check open files.
  - **Rust** (`Cargo.toml` at the root): starts rust-analyzer, whose `cargo check` covers every file, again on each save.
  - **Go** (`go.mod`): starts gopls, which checks every package of the module.
  - **Python** (`pyproject.toml`, `setup.py`, `setup.cfg`, `requirements.txt`): pyright/basedpyright run in workspace mode (ren answers their `python` / `basedpyright` configuration with `analysis.diagnosticMode: "workspace"`), so they check every file and keep it live as you edit. They're started only when one of them is the project's Python server. When ruff is one of the servers, `ruff check --output-format json --no-cache` runs too, with the severities ruff's server uses (syntax errors, F821, E902, E999 are errors, the rest warnings).
  - **TypeScript** (`tsconfig.json`): `tsc --noEmit --pretty false --incremental` from `node_modules/.bin` (or `PATH`), with its build info in the temp folder so reruns only recheck what changed. Without the incremental flags when the project refuses them.
  - A server that isn't installed stays quiet here; opening a file of that language still says so.
- **Cost.** ren itself doesn't slow down: servers and checkers are separate programs, and ren only reads their output on its 20 ms loop. The machine pays: rust-analyzer's first index and `cargo check`, pyright reading every file, and `tsc` on a big project can take seconds and a lot of memory. So:
  - The command-line checkers (ruff, tsc) run in the background, one at a time, under `nice -n 10`.
  - They run again only after a save of a file of their language, or of `pyproject.toml`, `ruff.toml`, `setup.cfg`, `package.json`, `tsconfig*.json`, half a second after the saves stop. A rerun also waits at least as long as the last run took, so a checker never gets more than half the time.
  - They don't run on a typing pause: they read the files on disk, so they'd see nothing new, and the open file's problems already come live from its server.
- **Whose view counts.** An open file with a live server shows the server's diagnostics (they follow unsaved edits). The checkers' results count for files no server is watching. A checker that fails keeps its last results.
- `:scan` checks again now (and asks rust-analyzer for a new `cargo check`), e.g. after a `git pull`. `"scan": false` (globally or in a project's `.ren/settings.json`) turns it all off; for pyright it applies on `:lsp restart`. The running checker shows in the status bar (`tsc…`).
- Tested in tmux, with no file open: a Cargo crate (error in a nested module, warning in another), a Go module, a Python project with basedpyright and then with ruff alone (fixing a file and saving recoloured it after the tab closed, though ruff's server had cleared it), and a TypeScript project with `tsc` 7 (a fix plus save cleared the red).

### 2026-11-12: Red and yellow file names for problems
- **A file with errors shows its name in red, one with only warnings in yellow** (errors win when it has both), like VS Code. Both the tab title and the file tree name take the colour; the icon keeps its own.
- **Folders in the tree take the worst colour of everything inside them**, so a collapsed folder still shows that something in it is broken.
- The colours come from every diagnostic ren holds, including the ones rust-analyzer's `cargo check` reports for files that aren't open. Info and hint diagnostics don't count. Colours are the ones the bar and gutter already use for errors and warnings.
- **Closing a tab no longer drops that file's diagnostics.** rust-analyzer's `cargo check` results still hold for a closed file, and the tree should keep showing them. Servers that only check open files (pyright, the TypeScript server) clear theirs on didClose. ren's own "not in any crate" warning goes on close, since it's only rechecked for open files. `:lsp stop` / `:lsp restart` and a server exiting now clear that server's diagnostics for every file, not just the open ones.
- **A file opened from the command line through a symlink gets its real path,** like the tree's root (the working directory) already did. On macOS `ren /tmp/x/a.rs` gave the buffer `/tmp/x/a.rs` while the tree had `/private/tmp/x/a.rs`: two different files to ren, so the colours didn't match, and rust-analyzer was handed the same project twice.

### 2026-11-12: Comment markers per file type
- **Ctrl+/ uses each file type's own comment syntax** (`comments.rs`). The highlighter only knows ten languages, so Ctrl+/ did nothing in CSS, HTML, SQL, Dockerfiles… and YAML got the TOML/INI pair (`#` and `;`). A table by file name and extension now covers far more types; anything it doesn't list falls back to the highlighter's language.
  - `//` (with `/* */`): Rust, C/C++, Objective-C, JS/TS, Go, Java, Kotlin, Swift, C#, Scala, Dart, Groovy/Gradle, Protobuf, JSONC/JSON5, SCSS/Less, Solidity, shaders, D, Haxe. Zig/Odin `//` only. PHP `//` and `#`. F# `//` with `(* *)`. Terraform/HCL `#` and `//`.
  - `#`: Python, shell, Ruby, Perl, R, Nix, Elixir, YAML, TOML, conf/cfg, properties, CMake, Make, Crystal, CoffeeScript, Nim, GraphQL, Tcl, PowerShell, Julia, awk/sed, systemd units, `.env*`. By name: `Dockerfile`, `Containerfile`, `Makefile`, `CMakeLists.txt`, `Gemfile`, `Rakefile`, `Justfile`, `Procfile`, `Vagrantfile`, `Brewfile`, `Pipfile`, `.gitignore`, `.gitattributes`, `.dockerignore`, `.npmrc`, shell rc files.
  - `--`: SQL (with `/* */`), Lua (with `--[[ ]]`), Haskell/Elm/PureScript (with `{- -}`), Ada, VHDL.
  - `;`: INI/reg (and `#`), Lisp, Clojure, Emacs Lisp, Scheme, Racket, assembly. `%`: TeX/BibTeX, Erlang. `"`: Vim script. `'`: VB. `REM` / `::`: batch.
  - **Block only**, one block around the line or the selected lines: `/* … */` in CSS; `<!-- … -->` in HTML, XML, SVG, XAML, plist, Markdown, Vue/Svelte/Astro, MSBuild projects and templates (JSP, ERB, Handlebars, Nunjucks). Ctrl+/ again removes the block when the lines start with the open marker and end with the close one.
  - JSON has no comments and says so.
- Limit: Vue/Svelte/Astro files get `<!-- -->` everywhere, also inside `<script>`/`<style>`, where VS Code would switch to `//` or `/* */`. Noted in TODO.md.

### 2026-11-12: Ctrl+/ comments and uncomments lines
- **Ctrl+/** toggles a line comment on the cursor's line, or on every selected line. A selection ending at column 0 doesn't take that last line.
  - Only the marker at the start of the line, after its indent, counts: `// hello world // hello world` becomes `hello world // hello world`, and Ctrl+/ again puts it back.
  - When every non-blank line is commented, the markers come off, with one space after them. Otherwise each line gets `// ` (or `# `, `-- `: the file type's first marker) at the block's smallest indent, so the markers line up.
  - Blank lines are left alone. The cursor and selection stay on the same text. One Ctrl+Z undoes it.
  - Uncommenting knows every marker of the file type (e.g. `#` and `;`). Plain text and JSON have none and say so.
- Without kitty keys, terminals send Ctrl+/ as byte 0x1F, which crossterm reports as Ctrl+7 (Ctrl+_ on some). All three are taken. Tested in tmux with Rust (`//`) and Python (`#`).

### 2026-11-12: Rest the mouse on code: problems, info and quick fixes
- **Resting the mouse on code for a second opens a box** (`peek.rs`, drawn by `draw_peek`), top to bottom:
  - **Each problem at that spot,** worst first: `✖ error` / `▲ warning` / `ℹ info` in its colour, who found it with the rule code (`rustc E0308`, `Ruff F401`, `basedpyright reportUnusedImport`), the server's full message (every line, wrapped, not just the first line the bar shows), and the rule's docs link (`→ https://…`) when the server sends one.
  - **The server's hover** (type, signature, docs).
  - **Quick fixes** (`⚡ Import std::collections::HashMap`, `⚡ Qualify as …`). They highlight under the mouse and a click applies one. `Looking for quick fixes…` shows until the servers answer. ren's own fix (`Insert mod x;`) is listed on its warning.
- **When it shows:**
  - Only over actual text; not past the end of a line, and not while a menu, the completion list or the F1 box is open, or in Command mode.
  - With no problem at the spot it shows just the hover, and nothing when the server has nothing to say.
  - It sits below the line, or above when there's no room.
  - It stays open while the mouse is on the problem's span (or the word) or inside the box. It closes when the mouse leaves both, on a click outside, a scroll, any key, a text change or another tab.
- **Requests:** `textDocument/hover` at the spot, and `textDocument/codeAction` with `only: ["quickfix"]` and the problems' diagnostics to every server of the language that can, merged with duplicates dropped, at most 8 fixes (Ctrl+. still lists every action). Each box gets a number, so late answers for a closed box are ignored. Errors stay silent.
- Hover text from any server: markdown rules (`---`) now become a blank line instead of showing as dashes, in the F1 box too.
- Note: undeclared names (`HashMap` without its `use`) are reported by `cargo check` on save, not live by rust-analyzer. Their Import / Qualify fixes show in the box after a save.
- Testing note: tmux treats a trailing `;` in `send-keys` as a command separator, which looked like ren dropping `;` after `()`. It wasn't.

### 2026-11-12: Auto-import across modules works
- **Fixed: accepting a completion for something from another module didn't add its `use` line.** rust-analyzer offers items that aren't in scope yet (e.g. `deep_fn` from `utils::deep`), but works out their import only on `completionItem/resolve`. ren inserted the text first and resolved after, so the change reached the server first. rust-analyzer then redid the resolve against the changed text, didn't find the item and returned it without `additionalTextEdits` (seen in the LSP log).
  - Now ren resolves first, waiting up to 500 ms (`RESOLVE_TIMEOUT`) while the text is unchanged. Then it applies the completion and the import together. Accepting `deep_fn` gives `use crate::utils::deep::deep_fn;` at the top, and the cursor ends after `deep_fn()`.
  - A late answer is still applied when it arrives, as before.
- **The popup shows where an item comes from:** the server's `labelDetails.detail` is drawn dim after the name, e.g. `deep_fn() (use crate::utils::deep::deep_fn)`, like VS Code.
- Checked as well: the `Ctrl+.` quick fixes for an unresolved name (`⚡ Import crate::utils::deep::DeepThing`, `⚡ Qualify as utils::deep::DeepThing`) work. The import merges into an existing `use` (`use crate::utils::deep::{DeepThing, deep_fn};`).
- Still Rust's rules: only items the module can see are offered (`pub` items in `pub mod`s), and only from files that are in the crate (see the entry below).

### 2026-11-12: Rust files outside the module tree, several projects per server
- **Why completion only worked in some files:** rust-analyzer only analyses files reached from a crate root (`main.rs`, `lib.rs`, …) through `mod name;` lines. A file nobody declares gets no completion, inlay hints or errors. This is typical of new files made with `:touch`. VS Code behaves the same but warns; rust-analyzer didn't send ren any warning (no diagnostic for the file), so it looked broken. Files that are declared worked in every module (checked: `utils/mod.rs` opened first or second, and module files in ren's own source).
- **ren now checks** with rust-analyzer's `experimental/parentModule`: null means no crate contains the file.
  - When it is asked: for the open Rust file, once the server has been idle for 1.5 s and typing has paused, and again whenever any Rust file changes.
  - Crate roots are never flagged: `main.rs`, `lib.rs`, `build.rs`, `src/bin`, `examples`, `tests`, `benches`. Loading a project comes in steps with short gaps, which once gave a false warning for a `lib.rs`.
  - A file outside the tree gets a warning on line 1 (gutter, ▲ count, `:diag`, the bar), e.g. `Not in the crate: add mod orphan; to src/main.rs (Ctrl+. or :mod)`.
- **Fix it in one step:** `Ctrl+.` lists `⚡ Insert mod orphan; in src/main.rs` (per possible parent: `lib.rs`/`main.rs` next to it, `x.rs` or `x/mod.rs` for a folder). `:mod [parent]` does the same.
  - The line goes after the last top-level `mod x;`, or at the top below `//!` docs and `#![…]`.
  - The parent opens in a background tab, edited but not saved. The warning clears as soon as rust-analyzer sees the line.
  - rust-analyzer offers its own "Insert `mod`" fixes too, but only with the cursor on line 1.
- **Several projects, one server:** ren ran one server per language, rooted at the first file's project, so files from a second Cargo project got nothing. Now opening a file outside the running server's folders sends `workspace/didChangeWorkspaceFolders` to add that project, and the bottom bar says so. `workspace/workspaceFolders` answers with all of them.

### 2026-11-12: Tab completion on the `:` line
- **Fixed: Tab looked dead in `:touch`, `:mkfile`, `:mkdir` (and every path command).** Two causes:
  - With several matches, the list went into a bar message. Since the `:` line moved into the bottom bar, messages aren't shown while typing a command, so nothing seemed to happen.
  - Relative paths start from the folder selected in the tree (`here()`). With an empty folder selected, there were no matches, and nothing said where it had looked.
- **Now a list opens above the bar,** under the word being completed. It is headed by where the matches came from (`in /src/`, `in ~/x/`) or by `nothing matches "…" in /scripts/` (red).
  - Folders come first (blue, ending in `/`); dot files only when the typed part starts with `.`.
  - The common start of all matches goes in. Tab / Shift+Tab / ↑↓ then step through the list, filling each match in. Typing goes on from there (e.g. into a folder), and Esc closes only the list.
- **Tab also completes command names** (`:to` → `:touch `) **and fixed first words**: `:hints on|off|types…`, `:lsp restart|stop`, `:settings project`, `:terminal new|close`, `:containers start|stop|…`, `:debug config|stop|…`, `:recover discard`, `:install codelldb|debugpy|delve`. When no word matches, paths are offered (`:debug ./target/…`).

### 2026-11-12: Inlay hint kinds are settings
- **`inlayHints` is now an object** with one switch per kind: `enabled`, `types`, `parameters`, `returnTypes`, `chaining`, `closingBraces` (all `true` by default). `"inlayHints": false` still works as short for `enabled`. The project file can override single keys.
- **`:hints` opens a checkbox menu** (`[x]`/`[ ]`, like the git panel): Enter switches the row's kind and the menu stays open on it. `:hints on|off|types|params|returns|chaining|braces` does it from the command line. Both are for the session; the bottom bar says to keep it in settings. Switching a kind off hides it at once; on asks the server again.
- **How kinds are told apart:** LSP only marks hints as type (1) or parameter (2). ren sorts the rest itself:
  - a label starting with `->`, or `: T` right after `)` (TypeScript), is a return type;
  - a type hint at the end of a line is a chain type;
  - an unmarked hint after `}` is a closing brace.
  Servers are still asked for every kind; filtering happens in ren, so it works the same for servers ren can't configure (clangd).
- **Fixed: hints could stay partial after rust-analyzer finished indexing.** An early answer had `{unknown}` types, and no refresh came after the last indexing pass. Now hints are asked for again whenever a server finishes its background work (its progress list empties), like on `workspace/inlayHint/refresh`.
- **Fixed: rust-analyzer's `" -> i32"` lost its leading space** (`|x: i32|-> i32`). Label clean-up now keeps edge spaces and only collapses runs of whitespace.

### 2026-11-12: Inlay hints, tab line fix
- **The open tab's primary line touches its label.** It was a `▔` at the top of the padding row, which left a row of empty space before the text. Now it is a `▁` at the bottom of that row.
- **Inlay hints** (`textDocument/inlayHint`): types and parameter names from the language server, drawn in the code as dim italic text on a faint box. Examples: `let total: i32 = add(a: 1, b: 2)`, chain types, `for i int, v string := range` in Go. They're not part of the file: the cursor, clicks, selection and sideways scrolling all step over them. A click on a hint lands on its spot.
  - When: asked for after typing pauses for 250 ms, for the open tab, the whole file. Also asked for again when the server says its hints changed (`workspace/inlayHint/refresh`), which rust-analyzer does once it has finished indexing.
  - While typing: hints move with the edits until the new answer arrives (`Buffer::shift_hints`). Typing at a type hint's spot pushes it (`let x|: i32` → `let xy: i32`). At a parameter hint the text goes after it. Hints inside deleted text go. Undo, replace-all and other whole-line swaps drop the affected hints until the answer comes back. The editor's own typing, backspace, delete, tab and auto-dedent now go through `insert_text`/`delete_text` so they're tracked too.
  - Servers: rust-analyzer, clangd and basedpyright send hints by default. ren turns on gopls's (all off by default) and TypeScript's (`includeInlay…` preferences, incl. function return types). For rust-analyzer it adds closure return types for closures with a block body. pyright (not based) has no inlay hints.
  - `"inlayHints": true` in settings; `:hints` toggles for the session.
  - Note: the `pub fn add(a: i32, b: i32) { return a + b; }` example from the request gets no hint in Rust. A fn without `-> T` returns `()`, so rust-analyzer reports a type mismatch instead (VS Code shows the same). The `: i32` after a parameter list is TypeScript's "function return type" hint, which is now on for TS.

### 2026-11-12: Taller tabs, close × on tabs, `:w` / `:wq`, capped sideways scroll
- **The tab bar is two rows:** a padding row on top, the labels below. The open tab takes the editor's background, with a thin primary line (`▔`) across its top. The tree, editor and panel start one row lower.
- **Close a tab from the tab bar:** hovering a tab turns its icon into a red ×. Clicking × (or middle-clicking the tab) closes a saved tab straight away. With unsaved changes it switches to that tab and opens the `:` line, with a dim hint (`q! to discard, wq to save`) that disappears as you type. Every tab now has the icon slot, also untitled ones and with `icons` off (blank there), so nothing shifts on hover.
  - Decision: × on the last tab doesn't quit ren (it leaves an empty tab, like the middle click already did). A typed `:q`, `:q!` or `:wq` still quits on the last tab, as before.
- **`:w` and `:wq` didn't exist.** Only `:save` did. Added `:w` / `:write` (same as `:save`, with an optional path) and `:wq` / `:x` (save, then close the tab; nothing closes if the save fails).
- **Sideways scrolling (trackpad / horizontal wheel) stops at the widest line on screen:** its end lands at the right edge, plus one column for the cursor. When nothing on screen is wider than the editor it doesn't scroll at all.
  - Before, it scrolled without limit and pulled the cursor along. When the cursor's line was short, the view jumped back to it on the next mouse event. Now the cursor follows while its line reaches into the view. On a shorter line it stays put, off screen, and the view no longer jumps back. The next key or click brings the view back to the cursor.

### 2026-11-11: One bottom bar
- **The `:` command line moved into the status bar,** which now sits on the terminal's last row with nothing under it. The editor, tree and bottom panel gain that row. While typing a command the bar shows `COMMAND :…`; the position/language info on the right stays while the command is short enough.
- **Messages and the diagnostic under the cursor** (both used to have the separate bottom row) show in the bar: a message takes the file name's place until the next key; a diagnostic follows the file name, coloured by severity, first line only. Info messages are bright, errors red.

### 2026-11-11: Panel layout, primary dividers, closing terminals
- **The bottom panel sits under the editor only.** It starts at the file tree's divider instead of taking the full width, so the tree (or the git panel) keeps its full height. The panel draws on its own canvas and is copied in (`Screen::blit`); click targets are in panel coordinates. Terminals are sized to the panel's width.
- **Dividers are always the primary colour:** the tree/git divider (before, only while the tree had focus), and the column dividers in the Debug and Containers tabs.
- **Kill a terminal from its tab:** hovering a terminal tab shows a red ×; clicking it (or middle-clicking the tab) kills that shell. The × always has room reserved, so tabs don't shift. Closing the last terminal hides the panel instead of starting a new shell right away; `:terminal close` does the same.
- **Narrow panel headers:** the Debug and Containers buttons drop their key hints / labels (`▶ ⤼ ↓ ↑ ⟲ ■ ✕`) when they don't fit; the Debug status and `config ▾` get clipped instead of pushing the buttons off screen.

### 2026-11-11: Syntax colours inside strings
Wish list 10. The highlighter (`highlight.rs`) now reads string contents instead of painting the whole literal green.
- **Three new colours:** escapes (`\n`, `\x41`, `\u{…}`, `\N{…}`, octal, and `{{` `}}` / `%%`) in cyan; placeholders in the normal text colour. String text, the quotes and prefixes stay green (the quotes were white at first; changed the same day).
- **Placeholders per language:**
  - Rust: `{}` `{name}` `{0}` `{:?}` `{x:>8}` in every string, raw ones too. `{ x }`, `{"a": 1}` and a lone `{` stay plain text.
  - Python: f-strings `{expr}` (`{user.name!r}`); plain strings `{}`/`{name}` (for `.format`) and `%s` / `%(name)d`.
  - JS/TS: `${expr}` in template literals.
  - Go, C, Lua: printf verbs (`%d`, `%-5.2f`, `%+v`, `%zu`).
  - Shell: `$var`, `${var}`, `$(cmd)` in double quotes.
- **Strings that span lines** now keep their colour on the next lines: Rust strings and `r#"…"#`, Python `"""`/`'''`, JS template literals, Go backticks, TOML triple quotes. Before, only `/* */` comments carried over, so a Python docstring was coloured as code.
- **Raw strings have no escapes:** Rust `r"…"`, Python `r"…"`, Go backticks, shell `'…'`.
- **Git diff:** each side is highlighted as a whole file, so multi-line strings and comments are coloured right there too (removed from TODO.md).
- **Decided myself:** prefixes (`r#`, `f`, `b`) take the quote colour; the escape colour is One Dark's cyan, the same as macros.

### 2026-11-11: Rebuild from the wish list
The whole wish list above, built in one go. Vim is gone: every old keybinding and command it replaced was deleted, not kept alongside. ~10.8k → ~15.9k lines of Rust (tests included). One new crate: `vt100` (terminal screen, Unix only); the pseudo-terminal comes from `libc`, which was already a dependency.

- **Modes** (`editor.rs` rewritten): `Edit` + `Command`. Typing edits; Shift+arrows, Ctrl/Alt+←/→ (word), Ctrl+Home/End, mouse drag/double/triple click select; Ctrl+A/C/X/V/Z/Y/S. No selection + Ctrl+C/X = the whole line. Tab/Shift+Tab indent the selected lines. Typing runs and deleting runs are one undo step each.
  - **Ctrl+Esc** swaps Edit ↔ Command. ren turns on the kitty keyboard protocol when the terminal supports it (`main.rs`; `REN_NO_KITTY=1` turns it off). Without it, Esc opens the `:` line once there is nothing left to close (popup, selection, find bar).
  - Removed: Normal/Insert/Visual/VisualLine/Search modes, motions and operators, `/` search, `:s///`, `:w`, `:wq`, `:x`, `:wa`, `:e`, `:e!`, `:enew`, `:bn`/`:bp`/`:bd`/`:b`, `:ls`, `:set …`, relative numbers, `:noh`, `:clip`. Kept: `:q`, `:q!`, `:qa`, `:qa!`, `:<line>`, `:!cmd`.
  - Added to keep things reachable: `:save [path]` (save as), `:revert` (was `:e!`), `:open <path>`, F1 hover, F8/Shift+F8 diagnostics, Ctrl+G opens `:`, Ctrl+PgUp/PgDn tabs.
- **File commands** (`fileops.rs`, `files.rs`): `:touch`/`:mkfile`, `:mkdir`, `:cp`, `:mv`, `:duplicate`, `:rm`, `:bin [empty|undo|redo|gitignore]`. Paths: `/x` workspace root, `~/x` home, `../` walks out, otherwise relative to the tree selection's folder or the active file's folder. Quotes optional. Tab completes paths.
  - `:touch` on an existing file updates its modified time (Unix behaviour) and says so. A single new file opens and gets focus.
  - `.bin/`: `:rm` and anything `:cp`/`:mv` would overwrite are moved into `.bin/<slot>/`. The history (100 ops) lives in `.bin/index.json`, so Ctrl+Z / Ctrl+Y in the tree undo/redo file commands across restarts. First time `.bin` appears in a git repo, ren asks to add it to `.gitignore`. `.bin` is hidden in the tree.
  - Open tabs follow `:mv`; clean tabs of removed files close.
- **File tree**: arrows only (letters jump to names). Space previews (italic tab, one preview slot, focus stays in the tree); Enter opens a normal tab; editing or saving a preview makes it normal. Delete / F2 prefill `:rm` / `:mv`.
- **Icons** (`icons.rs`): Nerd Font glyph + colour per file type in the tree and tabs. `"icons": true` by default; `:icons` toggles for the session.
- **Settings** (`settings.rs`): project `.ren/settings.json` merges over `~/.ren/settings.json` (objects by key, arrays/values replace); JSONC allowed. New keys: `icons`, `lsp.servers`, `lsp.format`, `containers.socket`, `git.templates`. `:settings project`.
- **`:reload` and crash recovery** (`session.rs`): `:reload` writes the session (tabs, cursors, scroll, preview flags, tree expansion/selection, panel) and the text of unsaved tabs to `~/.ren/session/<workspace>/`, re-execs ren with `--session` and restores it. Unsaved tabs are also copied there every 3 s; a clean quit deletes the copies, and after a crash the next start points at `:recover`.
- **Bottom panel** (`panel.rs`): Ctrl+` (kitty) or `:panel`; tabs TERMINAL | DEBUG | CONTAINERS (click, or Ctrl+PgUp/PgDn while focused).
  - **Terminal** (`term.rs`): login `$SHELL` on a pseudo-terminal in the workspace root, several sessions (`+`, `:terminal new [cmd]`, `:terminal close`), colours, cursor, title, scrollback (Shift+PgUp/PgDn, wheel), bracketed paste, app-cursor keys. Ctrl+↑ goes back to the editor.
  - **Debug**: the existing debugger, now a tab.
  - **Containers** (`containers.rs`): `docker ps -a` every 4 s with `DOCKER_HOST` from `containers.socket`; status by colour and icon (● running, ◐ paused/restarting, ○ stopped, ✖ dead/unhealthy); logs follow on the right; ▶ start / ■ stop / ⟲ restart / >_ shell buttons and `:containers …`; the shell opens `docker exec -it <id>` (bash, else sh) in a terminal tab.
- **Git panel** (`git.rs`, `git_ui.rs`): `:git` swaps the tree for STAGED / CHANGES with checkboxes; Space stages/unstages a file or a whole section; arrows preview a side-by-side diff (old left red, new right green, syntax highlighted); → / Tab into the diff, Shift+↑/↓ selects lines, Space stages/unstages just those lines (or the block under the cursor) via `git apply --cached --recount`. Branch line → filterable branch list (+ new branch). Buttons ✓ Commit, ↑ Push (sets the upstream the first time), ↓ Pull (both in the background), ⊟ Stash (list → pop/apply/drop). `:git init|add|reset|commit [msg]|push|pull|stash …|branch [name]|checkout [-b] name`. Checkout refuses with unsaved tabs and reloads clean tabs after. `:git init` writes `.gitignore` (`.bin/`, OS/editor files, `.env*` except `.env.example`, logs, and Rust/Node/Python/Go entries from what's in the folder).
  - **Commit form** from `git.templates` (ren's own format: `format` lines + `fields` with `label`, `hint`, `prefix`/`suffix`, `choices`, `free`, `lines`, `required`). Empty fields drop their prefix/suffix, spaces collapse, trailing blank lines go. `ticket` is pre-filled from the branch name. Ctrl+T switches template, Ctrl+S commits. No templates → one "Message" box.
- **Debugger**: F5 lists the `.vscode/launch.json` configurations when there are several (also `:debug config` and the header's `config ▾`), and remembers the pick. `${env:NAME}`, `${workspaceFolderBasename}`, `${userHome}`, `${cwd}` substituted. Python (auto plan and launch.json) runs with the project's `.venv` interpreter.
- **Language servers**: looked up in the project first (`.venv/bin`, `venv/bin`, `node_modules/.bin`, `$VIRTUAL_ENV`), then `PATH`. Several servers per language from `lsp.servers` (default: the old single first-found server). Servers are keyed per server; file notifications go to all of them; requests to the first ready one with the capability (`lsp.format` picks the formatter); diagnostics and code actions are merged (each server gets its own diagnostics as code-action context). Status bar shows `basedpyright-langserver + ruff`.
- Docs: `:help` rewritten, README (features, keys, commands, configuration, layout), Cargo description, PR template.
- Verified:
  - `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (34 pass: args, words, file ops + undo across reload, settings merge/templates/JSONC, icons, terminal keys + a real pty command, docker ps parsing, git status v2, diff rows, partial patches, commit messages, and a real-repo stage/unstage-by-line round trip).
  - Driven in tmux (sandbox `HOME`): editing/selection/cut/paste/undo, Esc → `:`; `:touch` with parents + `.gitignore` prompt, `:cp`, `:rm` + Ctrl+Z in the tree, `:bin`, Tab completion, `:duplicate`; Space preview / Enter; `:reload` keeping an unsaved edit; basedpyright + ruff from a Python project's `.venv` together (merged diagnostics, ruff quick fixes in the code-action menu, hover from basedpyright, `:format` via ruff); terminal (bash); containers via a stand-in `docker` script (no daemon was running); git panel, line staging, template commit (`[TKT-42] 🚀 add notes and fix two #time 15m`), checkout -b, branch picker, stash/pop, `:git init`.
  - Not verified live: the kitty-protocol keys (Ctrl+Esc, Ctrl+`) — tmux doesn't speak it; see TODO.md.

### 2026-11-11: `ruff server` support, rule codes in diagnostics
- **`ruff server`** added to the Python language-server candidates, after `pyright-langserver`, `basedpyright-langserver` and `pylsp` (one server per language still; see wish list 9). README updated.
- **Diagnostics show source and rule code** on their first line, e.g. `` `os` imported but unused [Ruff F401] `` and `Import "os" is not accessed [basedpyright reportUnusedImport]`. New `lsp::diag_message`, used by `on_diagnostics`.
  - Bug fixed: ren used to append `[source]` after the *last* line of the message. Ruff's messages have a second `help:` line, and every view (status line, `:diag` list, next/previous diagnostic) shows only the first line, so ruff's source never appeared.
  - The rule code (`code`, string or number) wasn't shown for any server.
  - When a server sends no `source`, its executable name is used.
- Verified:
  - `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` (20 pass, new `diag_tags`).
  - In ren on the test project copy: real `ruff server` picked when it's the only server (`[Ruff F401]`, `[Ruff I001]`), and basedpyright tagged with its rule names.

### 2026-11-11: Tested Python on a real project: LSP and debugger
No code change. I ran ren in tmux on a copy of a Python project in `/tmp`, with a throwaway `HOME`, so the real project, `~/.ren` and the tools folder weren't touched. The test file used shapely from the project's `.venv` (Python 3.14).
- **No venv on `PATH`:** `Language server not found`, even though `.venv/bin` has `basedpyright-langserver` and `ruff`.
- **basedpyright** (venv on `PATH`): works. It started and showed diagnostics (strict mode from `pyproject.toml`: `Import "os" is not accessed`). Hover showed shapely's `Point` signature and docstring from the venv.
- **ruff server** (through a wrapper script, since ren doesn't know it): works.
  - Diagnostics follow the `pyproject.toml` rules (F401, I001).
  - Code actions list the fixes, and "Remove unused import" applied.
  - `:format` and format-on-save reformatted `radii=[1.0,2.0,   3.0]`.
  - As expected for ruff, hover says "No information" and go-to-definition says "Server does not support this".
- **Debugger with `.vscode/launch.json`** (`type: debugpy`, `python: ${workspaceFolder}/.venv/bin/python`): works.
  - On the first F5 it downloaded debugpy 1.8.22 by itself.
  - The F9 breakpoint stopped; F11 stepped into the function and F10 stepped over.
  - The variables tree showed `r = 1.0`, then `r = 2.0` on the next hit.
  - The watch `total * 10` updated, and evaluated as a `NameError` in a scope without that name, which is correct.
  - It ran to the end: the program printed `total 43.91`, `Program exited (code 0)`.
  - It used the first configuration; there's no picker yet.
- **Debugger without `launch.json`:** fails. It runs the system `python3`, so `ModuleNotFoundError: No module named 'shapely'`.
- These results feed wish-list item 9 and the gaps in item 8.

### 2026-11-11: `~/.ren` settings, primary colour, format on save
- **`~/.ren/settings.json`** (new `src/settings.rs`). JSON, because serde_json was already a dependency, so no new crate. Created by the installers (`ren --init`) and on first start if missing. `:settings` opens it, and saving it from inside ren re-applies the settings immediately. Missing keys fall back to defaults. Bad values show an error in the status line and use the default.
  ```json
  { "theme": { "primary": "#DA56D1" }, "formatOnSave": true }
  ```
- **Primary colour `#DA56D1`** is the new default, replacing ember. The `ACCENT` const became runtime `render::primary()` (an `AtomicU32`), so it can be changed without a restart.
- **Focused file tree uses the primary colour**: header, the `│` divider (was blue), the `TREE` status badge (was cyan), the selected row (a 30% primary tint, was grey), and the currently open file's name (was blue).
- **formatOnSave (default true)**: before writing, ren sends `textDocument/formatting` to the file's LSP and waits up to 1.5 s, handling LSP events meanwhile. Because the wait is synchronous, `:wq`/`:x`/`:wa` format too. If there's no server, no formatting support, or the server isn't ready, the file is written as-is. A timeout cancels the request and writes unformatted, so a save is never lost. The status line ends with `, formatted` or `, format timed out`. Tested with rust-analyzer.
- **State moved**: `~/.config/ren/config` → `~/.ren/state` (recent folders only). I moved my existing file by hand.
- Note: the app icon (`assets/ren.svg`) is still ember. It's static, so it doesn't follow `theme.primary`.

### 2026-11-11: Ghostty launcher + local install
- `install.sh` macOS: `Ren IDE.app` now opens ren in a new Ghostty window (`open -na Ghostty --args -e $SHELL -lic 'exec ren'`). The login + interactive shell is needed because my PATH (cargo, rust-analyzer) is set in `~/.zshrc`. Falls back to iTerm2, then Terminal.app.
- `install.sh` Linux: the `.desktop` entry uses `ghostty -e` when Ghostty is on PATH.
- `install.sh --local`: builds `target/release/ren` from the checkout and installs it (binary in `~/.local/bin`, plus the app and icon) without needing a GitHub release. The binary is swapped with an atomic `mv`, so a running ren keeps working while I reinstall.

### 2026-11-11: Cut i18n, Discord, flowchart and AI
~16.1k → ~10.8k lines of Rust. Eight `tree-sitter*` crates dropped.

- **Turkish / i18n removed.** `i18n.rs` and `help_tr.txt` deleted. All 192 surviving `t!("tr", "en")` / `tf!(…)` calls collapsed to plain English strings / `format!`. `help_en.txt` → `help.txt`. `:lang` command and `lang` config key gone. Internal buffer names are now English (`[untitled]`, `[help]`). The DAP `locale` is fixed to `en`. The UI used to default to Turkish when `LANG` wasn't set; it's English now.
- **Discord removed.** `discord.rs`, the `presence` field, `:discord`, the `discord`/`discord_client_id` config keys and the `REN_DISCORD_ID` env var.
- **Flowchart removed.** `flow.rs`, `flow_ui.rs`, `Ctrl+G`, `:flow`, `Focus::Flow`, the panel layout and drag modes, and every `tree-sitter*` dependency (only the flowchart used them).
- **AI removed.** `ai.rs`, `ai_ui.rs`, `Ctrl+L`, `:ai`, `Focus::Ai`, the side-panel layout, paste/mouse routing, the model/chat/provider menu actions (`MenuAction::{Model, Chat, AiProvider}`, `Menu.custom_model`), and the `ai_*` config keys.
- Docs cleaned: README features/keys/sections/config table/project layout, CONTRIBUTING (translation rule, help-file rule, structure table), `:help`.
- Still need a C compiler to build: `ring` (TLS for the debug-adapter downloader) compiles C.
- Old config keys (`lang`, `discord*`, `ai_*`) left in an existing config file are simply ignored.
- Verified: `cargo fmt --check`, `cargo clippy --all-targets -D warnings` (same as CI), `cargo test` (16 pass). Launched in a terminal: `:ai`, `:flow`, `:discord` and `:lang` all report "Unknown command".

### 2026-11-11: Rebrand zetsu → ren
- Crate and binary renamed `zetsu` → `ren`. Version stays `0.3.0` to track the fork point.
- Every `zetsu`/`Zetsu` string → `ren`/`Ren` (CLI, tab-bar brand, help, LSP `clientInfo`, DAP `clientID`). `ZETSU_LSP_LOG` → `REN_LSP_LOG`.
- Paths: config → `~/.config/ren/config`, tools → `~/.local/share/ren/tools` (Windows: `%APPDATA%\ren`, `%LOCALAPPDATA%\ren`). Old zetsu config is **not** migrated.
- **New logo.** `assets/ren.svg` is the source, rendered to `ren-512.png`, `ren-1024.png` and `ren.ico` (16–256px). Ember gradient with an `r.` mark.
- **Start screen.** `ren.` half-block wordmark, tagline "refined terminal IDE". Only the trailing dot is white (zetsu's rule also painted the `▄` inside letters white).
- **Accent colour.** `LOGO_GREEN` (`#3dd68c`) renamed to `ACCENT` and set to ember `#ff8a4c` (logo, focus borders, debug highlights, find matches). To revert, change the one line in `src/render.rs`.
- Installers and release CI point at `mirairoad/ren`.

Regenerate icons after editing the SVG:
```sh
cd assets && rsvg-convert -w 1024 ren.svg -o ren-1024.png && rsvg-convert -w 512 ren.svg -o ren-512.png
for s in 16 32 48 64 128 256; do rsvg-convert -w $s ren.svg -o /tmp/ico-$s.png; done
magick /tmp/ico-{16,32,48,64,128,256}.png ren.ico
```

---

## Inventory

| Feature | Files | Lines | Pulls in | Verdict |
|---|---|---:|---|---|
| Core editor (modes, motions, undo, input, layout) | `editor.rs`, `buffer.rs` | 3.3k | crossterm, unicode-width | |
| Rendering (diff redraw, theme colours) | `render.rs`, `screen.rs` | 1.0k | crossterm | |
| Syntax highlighting (hand-rolled lexer) | `highlight.rs` | 0.5k | none | |
| LSP client + IDE glue | `lsp.rs`, `ide.rs` | 2.6k | serde_json | |
| Debugger (DAP) + adapter auto-download | `dap.rs`, `debug_ui.rs`, `tools.rs` | 2.5k | ureq (TLS/ring), zip | |
| Find & replace | `find.rs` | 0.7k | regex | |
| File tree + recent folders | `tree.rs` | 0.1k | none | |
| Config (key=value file) | `config.rs` | 0.06k | none | |
| System clipboard | (in editor) | | arboard | |
| Distribution: installers + release CI | `install.sh`, `install.ps1`, `.github/workflows/build.yml` | | | **keep**: install/update on all my machines |
| Issue / PR templates | `.github/ISSUE_TEMPLATE/`, `pull_request_template.md` | | | |
| Turkish / i18n | | | | **cut** ✓ |
| Discord Rich Presence | | | | **cut** ✓ |
| Live flowchart | | | 8× tree-sitter | **cut** ✓ |
| AI panel + AI modules | | | | **cut** ✓ (different plans) |

### Releases: how install/update will work
1. Create `github.com/mirairoad/ren` (it must be **public**, because the installers download without auth), add it as `origin`, then push `main`.
2. Tag and push: `git tag v0.3.0 && git push origin v0.3.0`. CI runs fmt + clippy + tests, builds Windows/Linux(musl)/macOS arm64+x86_64, and publishes a GitHub Release.
3. On each machine: `curl -fsSL https://raw.githubusercontent.com/mirairoad/ren/main/install.sh | sh` (or `install.ps1` on Windows). Run it again to update to the latest release.

### Rebuild-from-ground-up candidates (to discuss)
- **Theme**: `theme.primary` is done. The other colours (syntax, bars, selection) are still `const`s in `render.rs`. Grow `theme` into a full palette?
- **Keymap**: replaced by the **Wish list** above (Unix-style, no vim). Bindings are still scattered through `editor.rs` matches today, so the rebuild should put them in a single table.
- **Config**: done as `~/.ren/settings.json`. Per-project `.ren/settings.json` merging is on the wish list (6).
- **AI**: whatever the "different plans" are. Design it from scratch rather than reviving the old panel.

### Open questions for me
- Which languages do I actually edit in ren? (This decides LSP/DAP scope.)
- Keep Windows/Linux builds in CI, or macOS only?
- Keep the issue/PR templates and CONTRIBUTING (public project), or trim to a personal repo?
- Upstream sync: keep cherry-picking zetsu fixes, or diverge fully?
