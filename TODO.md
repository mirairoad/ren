# TODO — ren

What's left after the wish-list rebuild (CHANGES.md, Log 2026-11-11). Ordered roughly by how much it matters day to day.

## Check in Ghostty (couldn't be tested here)
- [ ] **Ctrl+Esc** swaps Edit ↔ Command, and **Ctrl+`** toggles the bottom panel. Both rely on the kitty keyboard protocol, which ren turns on when the terminal reports support. All testing ran in tmux, which doesn't speak it, so only the fallback (Esc opens `:`) was exercised. If something misbehaves, `REN_NO_KITTY=1 ren` forces the fallback.
- [ ] Nerd Font icons render in the tree and tabs (the codepoints are the common Seti/devicons ones; a wrong glyph is a one-line fix in `src/icons.rs`).
- [ ] Ctrl+Backspace / Ctrl+H: in terminals without kitty keys both arrive as the same byte, so one of "delete word" / "find and replace" can't be told apart. Alt+Backspace always deletes a word.

## Debugger
- [ ] `attach` configurations in launch.json (only `launch` runs today).
- [ ] `preLaunchTask` (needs `.vscode/tasks.json` support).
- [ ] `${input:…}` variables (prompt on the `:` line).
- [ ] JS/TS/Deno debugging (`node`, `pwa-node` types): Node and Deno projects can't be debugged yet.
- [ ] *(suggested)* Keep breakpoints and watches per project in `~/.ren/session/<workspace>/`.

## Git
- [ ] Partial (line) staging for new/untracked files: today Space stages the whole file.
- [ ] Merge conflicts: listed with `U`, but there's no resolve view.
- [ ] *(maybe)* One-column (inline) diff option; log/history; blame (not on the wish list).

## Bottom panel
- [ ] Terminal on Windows (no pseudo-terminal there yet; the tab shows an error).
- [ ] Containers: talk to the Docker API on the socket directly, so the `docker` CLI isn't required (today it runs the CLI with `DOCKER_HOST`). Not tested against a real daemon (none was running); it was tested with a stand-in `docker` script.
- [ ] Terminal: mouse reporting to programs that ask for it (e.g. `htop` clicks), and selecting/copying terminal text.

## Inlay hints
- [ ] TypeScript/JavaScript hints are untested (`typescript-language-server` isn't installed here). ren sends the inlay preferences both as `initializationOptions.preferences` and with `workspace/didChangeConfiguration`; if no hints show, that's where to look (`REN_LSP_LOG`).
- [ ] *(suggested)* Ctrl+click on a hint's type to go to its definition (servers send locations for label parts), and hover tooltips (`inlayHint/resolve`).
- [ ] *(suggested)* Ask only for the visible lines in very large files (today: the whole file, for the open tab).

## Problem colours
- [ ] *(suggested)* A problem count after red/yellow names in the tree (VS Code shows `2` for files and `●` for folders).
- [ ] Files opened while ren runs (`:open`, go to definition) through a symlink keep the symlinked path; only the command-line file is resolved.
- [ ] Project scan only looks for markers at the workspace root: a monorepo with `Cargo.toml` / `tsconfig.json` / `pyproject.toml` one level down isn't scanned until a file there is opened (and then only by servers that check the whole project).
- [ ] TypeScript solution-style roots (`"files": []` with `references`) give `tsc -p` nothing to check; they'd need `tsc -b`, which writes build output.
- [ ] Changes made outside ren (`git pull`, another editor) aren't noticed by ruff/tsc until the next save or `:scan`; rust-analyzer, gopls and pyright watch files themselves.
- [ ] *(suggested)* ESLint / Biome / mypy as checkers, and `deno check` for Deno projects.
- [ ] A checker still running when ren quits finishes on its own (it isn't killed).

## Projects
- [ ] A parked project's language servers aren't read until it comes back, so its colours and problems catch up on return. Polling them in the background means splitting the LSP handling from the editor state it updates.
- [ ] A breakpoint hit in a parked project doesn't announce itself (the session waits, stopped, until you switch back). *(suggested)* a mark on that project in the list.
- [ ] A project opened from the list starts fresh (tree open, no tabs); its last tabs come back only through `:reload`. *(suggested)* restore its last session.
- [ ] `Ctrl+K`, the welcome screen's folders and `:cd` still replace the current project's folder (the old behaviour) instead of opening a project; language servers already running keep their old root.
- [ ] *(suggested)* A key to step to the next / previous project without the list.
- [ ] `Ctrl+R` while the terminal has focus goes to the shell (its history search), not the project list.

## Sessions
- [ ] A panic saves nothing at that moment: the session is the last autosave (at most 3 seconds old).
- [ ] Terminals, debug sessions and language servers start fresh on restore; terminal scrollback isn't kept.
- [ ] Crash copies left in `~/.ren/session/` by an older ren are only picked up through a `:reload` from that ren, not listed.
- [ ] Only one session per set of projects is kept (plus ones with unsaved text, and windows interrupted within a minute of the newest); an older one for the same folders is deleted even though `ren --resume` could list it.
- [ ] Restoring a session into a ren that already saved its own deletes that ren's checkpoints (its open projects carry over into the restored session).
- [ ] *(suggested)* Show what a checkpoint had that the latest state doesn't (closed files, another project) in the `:session` list.
- [ ] *(suggested)* Restore the last session automatically when ren starts in the same folder (VS Code does), with a setting to turn it off.

## SQL, Dockerfile, C#, XML
- [ ] SQL: a table name right before `(` (`CREATE TABLE users (`) is coloured as a function call.
- [ ] Dockerfile: a continuation line (after `\`) that starts with an instruction's name colours it as one.
- [ ] C#: `""` inside a verbatim string `@"…"` ends and restarts it (same colours, but a verbatim string over several lines is cut there).
- [ ] XML has no language server: `lemminx` could be added for `.csproj` and friends. HTML isn't highlighted (the XML highlighter would trip over `<script>` contents).
- [ ] `dotnet build` needs exactly one solution (or one project) at the root; it doesn't look in subfolders.
- [ ] csharp-ls's latest release needs a newer .NET SDK than 8.0 (0.15 works with 8.0).
- [ ] *(suggested)* A notebook view for `.ipynb`: the cells as Python (or the notebook's language) instead of the raw JSON.

## Markdown
- [ ] The preview shows images as `[image: alt]` (no terminal graphics) and links aren't clickable.
- [ ] Inside a quote, lists and code blocks are rendered as plain paragraphs; HTML tables, footnotes and definition lists aren't rendered.
- [ ] Code blocks wider than the window are cut with `…` (no horizontal scroll).
- [ ] *(suggested)* Click a link in the preview to open it (a file in ren, a URL in the browser).
- [ ] *(suggested)* A live side-by-side preview while editing.

## Comments
- [ ] Ctrl+/ in Vue/Svelte/Astro: switch to `//` inside `<script>` and `/* */` inside `<style>` (today `<!-- -->` everywhere).

## Syntax colours
- [ ] Not covered yet: Lua long strings `[[…]]`, C++ raw strings `R"(…)"`, shell heredocs, Python 3.12 f-strings that reuse the outer quote inside `{…}`, and `{…}` / `${…}` expressions split over several lines.
- [ ] *(suggested)* LSP semantic tokens (rust-analyzer, basedpyright) for colours by meaning: parameters, mutable variables, enum members.

## Editing
- [ ] Undo groups break per typing/deleting run; VS Code also breaks at word boundaries and pauses.
- [ ] Ctrl+/ comment toggle, Alt+↑/↓ move line, Ctrl+D next occurrence (not on the wish list, but natural for this keymap).
- [ ] LSP hover when the mouse rests on a symbol (F1 only today).

## Windows build
- [ ] Ctrl+Esc opens the Start menu there: pick another key.
- [ ] File-command paths use `/`; check `\\` and drive letters.

## Open question
- [ ] Move `.bin` to `.ren/bin/` (one ren folder per project, only `.ren/bin/` gitignored)? Not done because it was never confirmed.

## Housekeeping
- [ ] `./install.sh --local` to put this build on PATH (the installed `ren` is still the pre-rebuild binary).
- [ ] Commit the work (nothing has been committed), then tag a release when ready.
