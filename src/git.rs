//! Git through the `git` CLI (so the user's config, credentials, hooks and
//! signing all apply): status, side-by-side diffs, partial staging, commit
//! messages built from templates, and the `.gitignore` written by `:git init`.

use crate::settings::Template;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn git(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C")
        .arg(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    c
}

/// Runs git and returns stdout, or stderr as the error.
pub fn run(root: &Path, args: &[&str]) -> Result<String, String> {
    let o = git(root).args(args).output().map_err(|e| format!("git: {}", e))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
        let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
        Err(if e.is_empty() { out } else { e })
    }
}

/// Runs git feeding `input` on stdin.
pub fn run_input(root: &Path, args: &[&str], input: &str) -> Result<String, String> {
    let mut c = git(root);
    c.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut ch = c.spawn().map_err(|e| format!("git: {}", e))?;
    if let Some(mut si) = ch.stdin.take() {
        let _ = si.write_all(input.as_bytes());
    }
    let o = ch.wait_with_output().map_err(|e| format!("git: {}", e))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
        Err(if e.is_empty() {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        } else {
            e
        })
    }
}

pub fn is_repo(root: &Path) -> bool {
    run(root, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|s| s.trim() == "true")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: String,
    pub orig: Option<String>,
    /// M modified, A added, D deleted, R renamed, C copied, ? untracked, U conflict.
    pub code: char,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub staged: Vec<Entry>,
    pub changes: Vec<Entry>,
}

/// Parses `git status --porcelain=v2 --branch -z`.
pub fn parse_status(out: &str) -> Status {
    let mut st = Status::default();
    let fields: Vec<&str> = out.split('\0').collect();
    let mut i = 0;
    while i < fields.len() {
        let f = fields[i];
        i += 1;
        if f.is_empty() {
            continue;
        }
        if let Some(h) = f.strip_prefix("# branch.head ") {
            st.branch = h.to_string();
        } else if let Some(u) = f.strip_prefix("# branch.upstream ") {
            st.upstream = Some(u.to_string());
        } else if let Some(ab) = f.strip_prefix("# branch.ab ") {
            for p in ab.split(' ') {
                if let Some(n) = p.strip_prefix('+') {
                    st.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = p.strip_prefix('-') {
                    st.behind = n.parse().unwrap_or(0);
                }
            }
        } else if let Some(p) = f.strip_prefix("? ") {
            st.changes.push(Entry {
                path: p.to_string(),
                orig: None,
                code: '?',
            });
        } else if f.starts_with("1 ") || f.starts_with("2 ") || f.starts_with("u ") {
            let renamed = f.starts_with("2 ");
            let n = match &f[..1] {
                "1" => 8,
                "2" => 9,
                _ => 10,
            };
            let parts: Vec<&str> = f.splitn(n + 1, ' ').collect();
            let Some(path) = parts.get(n) else { continue };
            let xy: Vec<char> = parts[1].chars().collect();
            let orig = if renamed {
                let o = fields.get(i).map(|s| s.to_string());
                i += 1;
                o
            } else {
                None
            };
            if f.starts_with("u ") {
                st.changes.push(Entry {
                    path: path.to_string(),
                    orig: None,
                    code: 'U',
                });
                continue;
            }
            if xy[0] != '.' {
                st.staged.push(Entry {
                    path: path.to_string(),
                    orig: orig.clone(),
                    code: xy[0],
                });
            }
            if xy.len() > 1 && xy[1] != '.' {
                st.changes.push(Entry {
                    path: path.to_string(),
                    orig,
                    code: xy[1],
                });
            }
        }
    }
    st
}

pub fn status(root: &Path) -> Result<Status, String> {
    run(
        root,
        &["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"],
    )
    .map(|o| parse_status(&o))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Same,
    Del,
    Add,
    /// A removed line shown next to the added line that replaced it.
    Mod,
}

/// One row of the side-by-side diff: old line on the left, new on the right.
/// `del`/`add` index the `-`/`+` lines of the patch, for partial staging.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub kind: RowKind,
    pub left: Option<(usize, String)>,
    pub right: Option<(usize, String)>,
    pub del: Option<usize>,
    pub add: Option<usize>,
}

/// A parsed single-file unified diff.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diff {
    pub header: Vec<String>,
    /// Hunks: (old start, new start, lines with their +/-/space prefix).
    pub hunks: Vec<(usize, usize, Vec<String>)>,
    pub rows: Vec<Row>,
    pub binary: bool,
}

fn hunk_starts(h: &str) -> (usize, usize) {
    // @@ -a,b +c,d @@
    let mut old = 1;
    let mut new = 1;
    for p in h.split(' ') {
        if let Some(r) = p.strip_prefix('-') {
            old = r.split(',').next().and_then(|n| n.parse().ok()).unwrap_or(1);
        } else if let Some(r) = p.strip_prefix('+') {
            new = r.split(',').next().and_then(|n| n.parse().ok()).unwrap_or(1);
        }
    }
    (old, new)
}

pub fn parse_diff(text: &str) -> Diff {
    let mut d = Diff::default();
    let mut in_hunk = false;
    for line in text.lines() {
        if line.starts_with("@@") {
            let (o, n) = hunk_starts(line);
            d.hunks.push((o, n, Vec::new()));
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            if line.starts_with("Binary files") || line.starts_with("GIT binary patch") {
                d.binary = true;
            }
            d.header.push(line.to_string());
            continue;
        }
        if line.starts_with('\\') {
            continue;
        }
        if let Some(h) = d.hunks.last_mut() {
            h.2.push(line.to_string());
        }
    }
    let mut seq = 0usize;
    for (hi, (o, n, lines)) in d.hunks.iter().enumerate() {
        let (mut ol, mut nl) = (*o, *n);
        if hi > 0 || ol > 1 {
            d.rows.push(Row {
                kind: RowKind::Same,
                left: None,
                right: None,
                del: None,
                add: None,
            });
        }
        let mut dels: Vec<(usize, String, usize)> = Vec::new();
        let mut adds: Vec<(usize, String, usize)> = Vec::new();
        let flush =
            |rows: &mut Vec<Row>, dels: &mut Vec<(usize, String, usize)>, adds: &mut Vec<(usize, String, usize)>| {
                let n = dels.len().max(adds.len());
                for k in 0..n {
                    let l = dels.get(k).cloned();
                    let r = adds.get(k).cloned();
                    let kind = match (&l, &r) {
                        (Some(_), Some(_)) => RowKind::Mod,
                        (Some(_), None) => RowKind::Del,
                        _ => RowKind::Add,
                    };
                    rows.push(Row {
                        kind,
                        left: l.as_ref().map(|x| (x.0, x.1.clone())),
                        right: r.as_ref().map(|x| (x.0, x.1.clone())),
                        del: l.map(|x| x.2),
                        add: r.map(|x| x.2),
                    });
                }
                dels.clear();
                adds.clear();
            };
        for l in lines {
            let (p, body) = l.split_at(l.len().min(1));
            match p {
                "-" => {
                    dels.push((ol, body.to_string(), seq));
                    ol += 1;
                }
                "+" => {
                    adds.push((nl, body.to_string(), seq));
                    nl += 1;
                }
                _ => {
                    flush(&mut d.rows, &mut dels, &mut adds);
                    d.rows.push(Row {
                        kind: RowKind::Same,
                        left: Some((ol, body.to_string())),
                        right: Some((nl, body.to_string())),
                        del: None,
                        add: None,
                    });
                    ol += 1;
                    nl += 1;
                }
            }
            if p == "-" || p == "+" {
                seq += 1;
            }
        }
        flush(&mut d.rows, &mut dels, &mut adds);
    }
    d
}

/// A diff "seq" index for every `-`/`+` line, in patch order.
fn change_seqs(d: &Diff) -> Vec<Vec<Option<usize>>> {
    let mut seq = 0;
    d.hunks
        .iter()
        .map(|h| {
            h.2.iter()
                .map(|l| {
                    if l.starts_with('-') || l.starts_with('+') {
                        seq += 1;
                        Some(seq - 1)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .collect()
}

/// A patch with only the `selected` changed lines (by seq index), for
/// `git apply --cached --recount` (or `--reverse` to unstage). Unselected
/// removals become context (or vanish when reversing); unselected additions
/// vanish (or become context when reversing). None when nothing is selected.
pub fn partial_patch(d: &Diff, selected: &[usize], reverse: bool) -> Option<String> {
    let seqs = change_seqs(d);
    let mut out = String::new();
    for h in &d.header {
        out.push_str(h);
        out.push('\n');
    }
    let mut any = false;
    for (hi, (o, n, lines)) in d.hunks.iter().enumerate() {
        let mut body = Vec::new();
        let mut changed = false;
        for (li, l) in lines.iter().enumerate() {
            let sel = seqs[hi][li].is_some_and(|s| selected.contains(&s));
            let (p, rest) = l.split_at(l.len().min(1));
            match (p, sel, reverse) {
                ("-", true, _) | ("+", true, _) => {
                    body.push(l.clone());
                    changed = true;
                }
                ("-", false, false) | ("+", false, true) => body.push(format!(" {}", rest)),
                ("-", false, true) | ("+", false, false) => {}
                _ => body.push(l.clone()),
            }
        }
        if !changed {
            continue;
        }
        any = true;
        let old = body.iter().filter(|l| !l.starts_with('+')).count();
        let new = body.iter().filter(|l| !l.starts_with('-')).count();
        out.push_str(&format!("@@ -{},{} +{},{} @@\n", o, old, n, new));
        for l in body {
            out.push_str(&l);
            out.push('\n');
        }
    }
    any.then_some(out)
}

/// Fills a commit template. Empty fields drop their prefix/suffix, doubled
/// spaces collapse, line ends are trimmed and trailing blank lines dropped.
pub fn build_message(t: &Template, values: &[String]) -> String {
    let mut lines = Vec::new();
    for fl in &t.format {
        let mut line = fl.clone();
        for (i, f) in t.fields.iter().enumerate() {
            let v = values.get(i).map(|s| s.trim_end()).unwrap_or("");
            let rep = if v.trim().is_empty() {
                String::new()
            } else {
                format!("{}{}{}", f.prefix, v, f.suffix)
            };
            line = line.replace(&format!("{{{}}}", f.key), &rep);
        }
        let multi = line.contains('\n');
        if multi {
            lines.extend(line.lines().map(|l| l.trim_end().to_string()));
            continue;
        }
        let mut collapsed = String::new();
        let mut prev_space = false;
        for c in line.trim().chars() {
            if c == ' ' && prev_space {
                continue;
            }
            prev_space = c == ' ';
            collapsed.push(c);
        }
        lines.push(collapsed);
    }
    let mut out: Vec<String> = Vec::new();
    for l in lines {
        if l.is_empty() && out.last().is_some_and(|p| p.is_empty()) {
            continue;
        }
        out.push(l);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// Ticket-like id in a branch name, e.g. `feat/TKT-000123-login` → `TKT-000123`.
pub fn ticket_from_branch(branch: &str) -> Option<String> {
    let re = regex::Regex::new(r"[A-Z][A-Z0-9]+-\d+").ok()?;
    re.find(branch).map(|m| m.as_str().to_string())
}

/// `.gitignore` lines for `:git init`: always `.bin/`, OS and editor clutter,
/// env files and logs, plus entries for the languages found in `root`.
pub fn default_ignores(root: &Path) -> Vec<&'static str> {
    let mut v = vec![
        "# ren",
        ".bin/",
        "",
        "# OS / editors",
        ".DS_Store",
        "Thumbs.db",
        "*.swp",
        ".idea/",
        ".vscode/",
        "",
        "# env and logs",
        ".env",
        ".env.*",
        "!.env.example",
        "*.log",
    ];
    if root.join("Cargo.toml").exists() {
        v.extend(["", "# Rust", "/target"]);
    }
    if root.join("package.json").exists() || root.join("deno.json").exists() {
        v.extend(["", "# Node / Deno", "node_modules/", "dist/"]);
    }
    let py = ["pyproject.toml", "requirements.txt", "setup.py"]
        .iter()
        .any(|f| root.join(f).exists());
    if py {
        v.extend([
            "",
            "# Python",
            "__pycache__/",
            ".venv/",
            "*.pyc",
            ".pytest_cache/",
            ".ruff_cache/",
        ]);
    }
    if root.join("go.mod").exists() {
        v.extend(["", "# Go", "/bin"]);
    }
    v
}

pub fn branches(root: &Path) -> Result<(Vec<String>, Vec<String>), String> {
    let out = run(
        root,
        &["for-each-ref", "--format=%(refname)", "refs/heads", "refs/remotes"],
    )?;
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for l in out.lines() {
        if let Some(b) = l.strip_prefix("refs/heads/") {
            local.push(b.to_string());
        } else if let Some(b) = l.strip_prefix("refs/remotes/") {
            if !b.ends_with("/HEAD") {
                remote.push(b.to_string());
            }
        }
    }
    Ok((local, remote))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Field;

    #[test]
    fn status_v2() {
        let out = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 M. N... 100644 100644 100644 h1 h2 src/a.rs\0\
1 .M N... 100644 100644 100644 h1 h2 src/b c.rs\0\
1 MM N... 100644 100644 100644 h1 h2 both.rs\0\
2 R. N... 100644 100644 100644 h1 h2 R100 new.rs\0old.rs\0\
? notes.txt\0";
        let s = parse_status(out);
        assert_eq!(s.branch, "main");
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (2, 1));
        let staged: Vec<&str> = s.staged.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(staged, vec!["src/a.rs", "both.rs", "new.rs"]);
        assert_eq!(s.staged[2].orig.as_deref(), Some("old.rs"));
        let changes: Vec<(&str, char)> = s.changes.iter().map(|e| (e.path.as_str(), e.code)).collect();
        assert_eq!(changes, vec![("src/b c.rs", 'M'), ("both.rs", 'M'), ("notes.txt", '?')]);
    }

    const DIFF: &str = "diff --git a/f.txt b/f.txt
index 1..2 100644
--- a/f.txt
+++ b/f.txt
@@ -1,4 +1,4 @@
 one
-two
-three
+TWO
+THREE
 four
";

    #[test]
    fn side_by_side_rows() {
        let d = parse_diff(DIFF);
        assert_eq!(d.rows.len(), 4);
        assert_eq!(d.rows[0].kind, RowKind::Same);
        assert_eq!(d.rows[1].kind, RowKind::Mod);
        assert_eq!(d.rows[1].left, Some((2, "two".into())));
        assert_eq!(d.rows[1].right, Some((2, "TWO".into())));
        assert_eq!((d.rows[1].del, d.rows[1].add), (Some(0), Some(2)));
        assert_eq!(d.rows[3].right, Some((4, "four".into())));
    }

    #[test]
    fn partial_patches() {
        let d = parse_diff(DIFF);
        // stage only "two" -> "TWO"
        let p = partial_patch(&d, &[0, 2], false).unwrap();
        assert!(
            p.contains("@@ -1,4 +1,4 @@\n one\n-two\n three\n+TWO\n four\n"),
            "{}",
            p
        );
        // unstage only the first removal: unselected additions become context
        let r = partial_patch(&d, &[0], true).unwrap();
        assert!(
            r.contains("@@ -1,5 +1,4 @@\n one\n-two\n TWO\n THREE\n four\n"),
            "{}",
            r
        );
        assert!(partial_patch(&d, &[], false).is_none());
    }

    #[test]
    fn messages() {
        let f = |key: &str, prefix: &str, suffix: &str| Field {
            key: key.into(),
            prefix: prefix.into(),
            suffix: suffix.into(),
            ..Default::default()
        };
        let t = Template {
            format: vec!["{ticket} {type} {summary} {time}".into(), "".into(), "{body}".into()],
            fields: vec![
                f("ticket", "[", "]"),
                f("type", "", ""),
                f("summary", "", ""),
                f("time", "#time ", ""),
                f("body", "", ""),
            ],
        };
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            build_message(&t, &v(&["TKT-1", "🩹", "fix login", "15m", "Details\nmore"])),
            "[TKT-1] 🩹 fix login #time 15m\n\nDetails\nmore"
        );
        assert_eq!(build_message(&t, &v(&["", "🩹", "fix login", "", ""])), "🩹 fix login");
        assert_eq!(
            ticket_from_branch("feat/TKT-000123-login").as_deref(),
            Some("TKT-000123")
        );
        assert_eq!(ticket_from_branch("main"), None);
    }

    #[test]
    fn real_repo_roundtrip() {
        let root = std::env::temp_dir().join(format!("ren-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        if run(&root, &["init", "-q"]).is_err() {
            return;
        }
        let _ = run(&root, &["config", "user.email", "t@example.com"]);
        let _ = run(&root, &["config", "user.name", "t"]);
        let _ = run(&root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("f.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        run(&root, &["add", "f.txt"]).unwrap();
        run_input(&root, &["commit", "-q", "-F", "-"], "first").unwrap();
        std::fs::write(root.join("f.txt"), "one\nTWO\nthree\nFOUR\n").unwrap();
        let d = parse_diff(&run(&root, &["diff", "-U100000", "--", "f.txt"]).unwrap());
        // stage just the first change (two -> TWO)
        let first: Vec<usize> = d
            .rows
            .iter()
            .find(|r| r.kind == RowKind::Mod)
            .map(|r| vec![r.del.unwrap(), r.add.unwrap()])
            .unwrap();
        let p = partial_patch(&d, &first, false).unwrap();
        run_input(&root, &["apply", "--cached", "--recount", "-"], &p).unwrap();
        let cached = run(&root, &["diff", "--cached"]).unwrap();
        assert!(cached.contains("+TWO") && !cached.contains("+FOUR"), "{}", cached);
        let st = status(&root).unwrap();
        assert_eq!(st.staged.len(), 1);
        assert_eq!(st.changes.len(), 1);
        // unstage it again, line by line from the staged diff
        let sd = parse_diff(&run(&root, &["diff", "--cached", "-U100000", "--", "f.txt"]).unwrap());
        let all: Vec<usize> = sd.rows.iter().flat_map(|r| [r.del, r.add]).flatten().collect();
        let rp = partial_patch(&sd, &all, true).unwrap();
        run_input(&root, &["apply", "--cached", "--recount", "--reverse", "-"], &rp).unwrap();
        assert!(run(&root, &["diff", "--cached"]).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
