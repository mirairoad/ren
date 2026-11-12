use super::*;
use crate::fileops::{self, Bin, Step};
use ide::MenuAction;

impl Editor {
    /// The `.bin` for the current workspace, loaded on first use.
    fn bin(&mut self) -> &mut Bin {
        let root = self.tree.root.clone();
        if self.bin.as_ref().is_none_or(|b| b.root != root) {
            self.bin = Some(Bin::open(&root));
        }
        self.bin.as_mut().unwrap()
    }

    fn rel_display(&self, p: &Path) -> String {
        match p.strip_prefix(&self.tree.root) {
            Ok(r) => format!("/{}", r.display()),
            Err(_) => p.display().to_string(),
        }
    }

    /// Handles the file commands. Returns false when `cmd` isn't one of them.
    pub(crate) fn file_command(&mut self, cmd: &str, args: &[String]) -> bool {
        let had_bin = self.tree.root.join(fileops::BIN).exists();
        match cmd {
            "touch" | "mkfile" => self.cmd_touch(args),
            "mkdir" => self.cmd_mkdir(args),
            "cp" => self.cmd_cp_mv(args, false),
            "mv" => self.cmd_cp_mv(args, true),
            "duplicate" => self.cmd_duplicate(args),
            "rm" => self.cmd_rm(args),
            "bin" => self.cmd_bin(args),
            _ => return false,
        }
        self.tree.refresh();
        if !had_bin && self.tree.root.join(fileops::BIN).exists() {
            self.ask_gitignore_bin();
        }
        true
    }

    fn cmd_touch(&mut self, args: &[String]) {
        if args.is_empty() {
            return self.error("Usage: :touch <path> …");
        }
        let mut made = Vec::new();
        for a in args {
            let p = self.resolve_path(a);
            match self.bin().touch(&p) {
                Ok(true) => made.push(p),
                Ok(false) => self.info(format!(
                    "{} already exists (modified time updated)",
                    self.rel_display(&p)
                )),
                Err(e) => return self.error(format!("touch {}: {}", a, e)),
            }
        }
        self.tree.refresh();
        if let Some(last) = made.last().cloned() {
            self.tree.reveal(&last);
            if made.len() == 1 {
                self.open_path(&last);
                self.focus = Focus::Editor;
            }
            let names: Vec<String> = made.iter().map(|p| self.rel_display(p)).collect();
            self.info(format!("created {}", names.join(", ")));
        }
    }

    fn cmd_mkdir(&mut self, args: &[String]) {
        if args.is_empty() {
            return self.error("Usage: :mkdir <path> …");
        }
        let mut made = Vec::new();
        for a in args {
            let p = self.resolve_path(a);
            match self.bin().mkdir(&p) {
                Ok(true) => made.push(p),
                Ok(false) => self.info(format!("{} already exists", self.rel_display(&p))),
                Err(e) => return self.error(format!("mkdir {}: {}", a, e)),
            }
        }
        self.tree.refresh();
        if let Some(last) = made.last().cloned() {
            self.tree.reveal(&last);
            let names: Vec<String> = made.iter().map(|p| self.rel_display(p)).collect();
            self.info(format!("created {}", names.join(", ")));
        }
    }

    fn cmd_cp_mv(&mut self, args: &[String], mv: bool) {
        let verb = if mv { "mv" } else { "cp" };
        if args.len() < 2 {
            return self.error(format!("Usage: :{} <src> … <dst>", verb));
        }
        let dst = self.resolve_path(&args[args.len() - 1]);
        let srcs: Vec<PathBuf> = args[..args.len() - 1].iter().map(|a| self.resolve_path(a)).collect();
        if srcs.len() > 1 && !dst.is_dir() {
            return self.error(format!("{}: {} is not a folder", verb, self.rel_display(&dst)));
        }
        let mut last = None;
        for src in &srcs {
            let r = if mv {
                self.lsp_close_under(src);
                self.bin().mv(src, &dst)
            } else {
                self.bin().copy(src, &dst)
            };
            match r {
                Ok(to) => {
                    if mv {
                        self.retarget_bufs(src, &to);
                    }
                    last = Some((src.clone(), to));
                }
                Err(e) => return self.error(format!("{} {}: {}", verb, self.rel_display(src), e)),
            }
        }
        self.tree.refresh();
        if let Some((src, to)) = last {
            self.tree.reveal(&to);
            let what = if srcs.len() > 1 {
                format!("{} items", srcs.len())
            } else {
                self.rel_display(&src)
            };
            let done = if mv { "moved" } else { "copied" };
            self.info(format!("{} {} → {}", done, what, self.rel_display(&to)));
        }
    }

    fn cmd_duplicate(&mut self, args: &[String]) {
        if args.is_empty() {
            return self.error("Usage: :duplicate <path> …");
        }
        let mut last = None;
        for a in args {
            let p = self.resolve_path(a);
            match self.bin().duplicate(&p) {
                Ok(to) => last = Some(to),
                Err(e) => return self.error(format!("duplicate {}: {}", a, e)),
            }
        }
        self.tree.refresh();
        if let Some(to) = last {
            self.tree.reveal(&to);
            self.info(format!("created {}", self.rel_display(&to)));
        }
    }

    fn cmd_rm(&mut self, args: &[String]) {
        if args.is_empty() {
            return self.error("Usage: :rm <path> …");
        }
        let mut n = 0;
        let mut name = String::new();
        for a in args {
            let p = self.resolve_path(a);
            self.lsp_close_under(&p);
            match self.bin().rm(&p) {
                Ok(()) => {
                    n += 1;
                    name = self.rel_display(&p);
                    self.close_missing_bufs(&p);
                }
                Err(e) => return self.error(format!("rm {}: {}", a, e)),
            }
        }
        self.tree.refresh();
        let what = if n == 1 { name } else { format!("{} items", n) };
        self.info(format!("moved {} to .bin (Ctrl+Z in the tree to undo)", what));
    }

    fn cmd_bin(&mut self, args: &[String]) {
        match args.first().map(String::as_str) {
            None => {
                let b = self.bin();
                let (items, u, r) = (b.items(), b.undo.len(), b.redo.len());
                let next = b.undo.last().map(|o| o.label.clone());
                let mut m = format!("{} items in .bin · {} undo, {} redo steps", items, u, r);
                if let Some(l) = next {
                    m += &format!(" · Ctrl+Z in the tree undoes \"{}\"", l);
                }
                self.info(m);
            }
            Some("empty") => {
                let n = self.bin().items();
                self.confirm(
                    &format!("Delete {} items in .bin for good?", n),
                    "Yes, empty .bin",
                    "bin empty!",
                );
            }
            Some("empty!") => match self.bin().empty() {
                Ok(n) => self.info(format!("Deleted {} items from .bin; file undo history cleared", n)),
                Err(e) => self.error(format!("bin empty: {}", e)),
            },
            Some("undo") => self.file_undo(),
            Some("redo") => self.file_redo(),
            Some("gitignore") => match fileops::gitignore_add(&self.tree.root, &[".bin/"]) {
                Ok(_) => self.info("Added .bin/ to .gitignore"),
                Err(e) => self.error(format!(".gitignore: {}", e)),
            },
            Some(other) => self.error(format!("Unknown :bin option: {} (empty, undo, redo, gitignore)", other)),
        }
    }

    /// A yes/no menu whose "yes" runs a `:` command.
    pub(crate) fn confirm(&mut self, title: &str, yes: &str, run: &str) {
        self.menu = Some(Menu {
            title: title.to_string(),
            items: vec![
                (yes.to_string(), MenuAction::Run(run.to_string())),
                ("Cancel".to_string(), MenuAction::Run(String::new())),
            ],
            sel: 1,
            scroll: 0,
            anchor: None,
            dir: None,
            filter: None,
            base: Vec::new(),
            ws_symbols: false,
        });
    }

    fn ask_gitignore_bin(&mut self) {
        let root = self.tree.root.clone();
        if self.gitignore_asked || !root.join(".git").exists() || fileops::gitignored(&root) {
            return;
        }
        self.gitignore_asked = true;
        self.confirm(
            "ren created .bin here. Add it to .gitignore?",
            "Yes, add .bin/",
            "bin gitignore",
        );
    }

    pub(crate) fn file_undo(&mut self) {
        let r = self.bin().undo();
        self.after_file_history(r, "undid", "Nothing to undo in .bin");
    }

    pub(crate) fn file_redo(&mut self) {
        let r = self.bin().redo();
        self.after_file_history(r, "redid", "Nothing to redo in .bin");
    }

    fn after_file_history(&mut self, r: std::io::Result<Option<fileops::Op>>, verb: &str, none: &str) {
        match r {
            Ok(Some(op)) => {
                for s in &op.steps {
                    let paths: Vec<PathBuf> = match s {
                        Step::Made { path, .. } | Step::Trashed { path, .. } => vec![path.clone()],
                        Step::Moved { from, to } => vec![from.clone(), to.clone()],
                    };
                    for p in paths {
                        let abs = self.tree.root.join(p);
                        self.close_missing_bufs(&abs);
                    }
                }
                self.tree.refresh();
                self.info(format!("{} \"{}\"", verb, op.label));
            }
            Ok(None) => self.info(none),
            Err(e) => self.error(format!("Could not {}: {}", verb.trim_end_matches('d'), e)),
        }
    }

    /// Closes the language-server view of buffers under `p` before it moves.
    fn lsp_close_under(&mut self, p: &Path) {
        for i in 0..self.bufs.len() {
            if self.bufs[i].path.as_ref().is_some_and(|bp| bp.starts_with(p)) {
                self.lsp_close_buf(i);
            }
        }
    }

    /// After `mv`: open tabs follow their files.
    fn retarget_bufs(&mut self, from: &Path, to: &Path) {
        for b in self.bufs.iter_mut() {
            let Some(bp) = b.path.clone() else { continue };
            if let Ok(rest) = bp.strip_prefix(from) {
                let np = if rest.as_os_str().is_empty() {
                    to.to_path_buf()
                } else {
                    to.join(rest)
                };
                b.name = np
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                b.lang = highlight::detect(&np);
                b.path = Some(np);
                b.lsp_open = false;
            }
        }
    }

    /// Closes clean tabs whose file under `p` no longer exists. Tabs with unsaved
    /// changes stay open (saving recreates the file).
    fn close_missing_bufs(&mut self, p: &Path) {
        let mut i = 0;
        let mut kept_dirty = false;
        while i < self.bufs.len() {
            let gone = self.bufs[i]
                .path
                .as_ref()
                .is_some_and(|bp| bp.starts_with(p) && !bp.exists());
            if gone {
                if self.bufs[i].dirty {
                    kept_dirty = true;
                } else {
                    self.lsp_close_buf(i);
                    self.bufs.remove(i);
                    if self.cur > i || self.cur >= self.bufs.len() {
                        self.cur = self.cur.saturating_sub(1);
                    }
                    continue;
                }
            }
            i += 1;
        }
        if self.bufs.is_empty() {
            self.bufs.push(Buffer::scratch(NONAME));
            self.cur = 0;
        }
        if kept_dirty {
            self.error("A removed file is still open with unsaved changes");
        }
    }
}
