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
