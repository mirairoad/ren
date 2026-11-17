//! What happens around a launch.json launch, as VS Code does it:
//!
//! - `${input:…}` / `${command:pickProcess}`: asked one at a time before
//!   anything runs (`promptString` on the `:` line, prefilled with the last
//!   answer or the default; `pickString` and the process list as menus).
//!   Esc cancels the launch. A restart reuses the answers.
//! - `preLaunchTask`: the task from `.vscode/tasks.json` (and what it
//!   `dependsOn`) runs in terminal tabs; the session starts when it exits
//!   with 0, or for a background task when its problem matcher's end pattern
//!   shows. A failure stops there (`:debug anyway` launches all the same).
//! - `postDebugTask`: run when the session ends, without waiting.
//! - `serverReadyAction`: the first output (Debug console, or the program's
//!   terminal) matching its pattern opens the URL in the browser.

use super::*;
use crate::launch::{Ask, Ctx};
use crate::tasks::{self, Task};
use debug_ui::DbgState;
use ide::MenuAction;
use serde_json::Value;
use std::time::Instant;

/// How long a background task gets to show it has started (VS Code: 5 s).
const BACKGROUND_START: Duration = Duration::from_secs(5);

/// A task running in a terminal tab.
pub struct Running {
    task: Task,
    pid: Option<u32>,
    /// Its matcher's begin pattern was seen (or `activeOnStart`).
    active: bool,
    started: Instant,
}

/// What runs after the tasks.
pub enum After {
    Launch(Box<crate::dap::LaunchPlan>),
    Nothing,
}

/// A `preLaunchTask` / `postDebugTask` with what it depends on: stages that
/// run one after the other, the tasks in a stage side by side.
pub struct TaskRun {
    label: String,
    stages: Vec<Vec<Task>>,
    running: Vec<Running>,
    after: After,
}

/// A configuration's `serverReadyAction`.
pub struct ServerReady {
    re: regex::Regex,
    uri_format: Option<String>,
    action: String,
}

impl ServerReady {
    pub fn new(v: &Value) -> Option<Self> {
        let pat = v["pattern"].as_str().unwrap_or(r"listening on.* (https?://\S+|[0-9]+)");
        let re = regex::RegexBuilder::new(pat).case_insensitive(true).build().ok()?;
        Some(ServerReady {
            re,
            uri_format: v["uriFormat"].as_str().map(str::to_string),
            action: v["action"].as_str().unwrap_or("openExternally").to_string(),
        })
    }

    /// The URL to open when `text` matches.
    fn url(&self, text: &str) -> Option<String> {
        let c = self.re.captures(text)?;
        Some(match c.get(1).map(|m| m.as_str()) {
            Some(cap) => {
                let fmt = match &self.uri_format {
                    Some(f) => f.clone(),
                    None if cap.chars().all(|c| c.is_ascii_digit()) => "http://localhost:%s".into(),
                    None => "%s".into(),
                };
                fmt.replacen("%s", cap, 1)
            }
            None => self.uri_format.clone().unwrap_or_default(),
        })
    }
}

/// Opens a URL in the system's browser (on WSL, Windows').
fn open_url(url: &str) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let wsl = std::env::var_os("WSL_DISTRO_NAME").is_some()
        || fs::read_to_string("/proc/version").is_ok_and(|v| v.to_lowercase().contains("microsoft"));
    let cmds: Vec<Vec<&str>> = if cfg!(target_os = "macos") {
        vec![vec!["open", url]]
    } else if wsl {
        vec![
            vec!["wslview", url],
            vec!["cmd.exe", "/c", "start", "", url],
            vec!["xdg-open", url],
        ]
    } else {
        vec![vec!["xdg-open", url]]
    };
    let mut last = std::io::Error::other("no browser opener");
    for c in cmds {
        match Command::new(c[0])
            .args(&c[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(_) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Running processes: (pid, command line), the user's own only with `mine`.
fn processes(mine: bool) -> Vec<(u32, String)> {
    let mut cmd = std::process::Command::new("ps");
    if mine {
        let user = env::var("USER").unwrap_or_default();
        cmd.args(["-U", &user, "-ww", "-o", "pid=,args="]);
    } else {
        cmd.args(["-axww", "-o", "pid=,args="]);
    }
    let Ok(o) = cmd.output() else { return Vec::new() };
    let me = std::process::id();
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let (pid, args) = l.split_once(' ')?;
            let pid: u32 = pid.parse().ok()?;
            (pid != me).then(|| (pid, args.trim().to_string()))
        })
        .collect()
}

impl Editor {
    /// What launch.json's variables refer to: the project on screen, the
    /// open projects (VS Code's workspace folders), the file and cursor.
    pub(crate) fn debug_ctx(&self) -> Ctx {
        let root = self.tree.root.clone();
        let b = self.buf();
        let settings = fs::read_to_string(root.join(".vscode").join("settings.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&crate::settings::strip_jsonc(&t)).ok())
            .unwrap_or(Value::Null);
        Ctx {
            folders: self.project_roots(),
            file: b.path.clone(),
            line: b.cy + 1,
            column: b.cx + 1,
            selected: self
                .selected_text()
                .map(|t| t.into_iter().collect())
                .unwrap_or_default(),
            answers: self.dbg.answers.clone(),
            python: None,
            default_build_task: tasks::read_tasks(&root).and_then(|d| tasks::default_build_task(&d)),
            settings,
            root,
        }
    }

    /// Asks for one value the launch needs; `:input <value>` (or picking
    /// from the list) answers it and the launch goes on.
    pub(crate) fn ask(&mut self, a: Ask) {
        self.dbg.visible = true;
        self.panel_tab = PanelTab::Debug;
        match a {
            Ask::Prompt(key, desc, default, password) => {
                let start = self.dbg.last_answers.get(&key).cloned().unwrap_or(default);
                self.dbg.asking = Some((key, password));
                self.prefill_cmd(&format!("input {}", start));
                self.cmd_hint = Some(format!("{} · Enter: go on · Esc: cancel", desc));
            }
            Ask::Pick(key, desc, opts, default) => {
                let mut items: Vec<(String, MenuAction)> = opts
                    .into_iter()
                    .map(|(label, value)| {
                        let mark = if default.as_deref() == Some(value.as_str()) {
                            "  (Default)"
                        } else {
                            ""
                        };
                        (
                            format!("{}{}", label, mark),
                            MenuAction::Run(format!("input {}", value)),
                        )
                    })
                    .collect();
                if let Some(i) = items.iter().position(|(l, _)| l.ends_with("(Default)")) {
                    let d = items.remove(i);
                    items.insert(0, d);
                }
                self.dbg.asking = Some((key, false));
                self.enter_edit();
                self.menu = Some(Menu::filterable(desc, items, 0));
            }
            Ask::Process(key, mine) => {
                let items: Vec<(String, MenuAction)> = processes(mine)
                    .into_iter()
                    .map(|(pid, args)| {
                        (
                            format!("{:>7}  {}", pid, args),
                            MenuAction::Run(format!("input {}", pid)),
                        )
                    })
                    .collect();
                if items.is_empty() {
                    return self.error("Could not list the processes (ps)");
                }
                self.dbg.asking = Some((key, false));
                self.enter_edit();
                self.menu = Some(Menu::filterable("Pick the process to attach to".into(), items, 0));
            }
        }
    }

    /// `:input <value>`: the answer to what the launch asked.
    pub(crate) fn input_answer(&mut self, value: &str) {
        let Some((key, password)) = self.dbg.asking.take() else {
            return self.error("Nothing is being asked");
        };
        if !password {
            self.dbg.last_answers.insert(key.clone(), value.to_string());
        }
        self.dbg.answers.insert(key, value.to_string());
        self.debug_go();
    }

    /// The task called `label` and everything it depends on, in stages.
    fn task_stages(
        &self,
        doc: &Value,
        label: &str,
        ctx: &Ctx,
        seen: &mut Vec<String>,
    ) -> Result<Vec<Vec<Task>>, crate::launch::Subst> {
        if seen.iter().any(|s| s == label) {
            return Err(crate::launch::Subst::Error(format!(
                "The task '{}' depends on itself",
                label
            )));
        }
        seen.push(label.to_string());
        let t = tasks::find(doc, label, ctx)?;
        let mut stages: Vec<Vec<Task>> = Vec::new();
        for d in &t.depends {
            let sub = self.task_stages(doc, d, ctx, seen)?;
            if t.sequence {
                stages.extend(sub);
            } else {
                // Side by side: the dependencies' stages merged level by level.
                for (k, st) in sub.into_iter().enumerate() {
                    match stages.get_mut(k) {
                        Some(s) => s.extend(st),
                        None => stages.push(st),
                    }
                }
            }
        }
        seen.pop();
        stages.push(vec![t]);
        Ok(stages)
    }

    /// Runs the `preLaunchTask`, then the plan.
    pub(crate) fn run_pre_task(&mut self, label: &str, plan: crate::dap::LaunchPlan) {
        if !std::mem::take(&mut self.dbg.keep_console) {
            self.dbg.console.clear();
        }
        self.dbg.visible = true;
        self.start_tasks(label, After::Launch(Box::new(plan)));
    }

    /// Runs the `postDebugTask` (nothing waits for it).
    pub(crate) fn run_post_task(&mut self, label: &str) {
        if self.dbg.tasks.is_some() {
            return;
        }
        self.start_tasks(label, After::Nothing);
    }

    fn start_tasks(&mut self, label: &str, after: After) {
        let root = self.tree.root.clone();
        let Some(doc) = tasks::read_tasks(&root) else {
            return self.task_failed(
                after,
                format!("Could not find the task '{}' (no .vscode/tasks.json).", label),
            );
        };
        let ctx = self.debug_ctx();
        let stages = match self.task_stages(&doc, label, &ctx, &mut Vec::new()) {
            Ok(s) => s,
            Err(crate::launch::Subst::Ask(a)) => return self.ask(a),
            Err(crate::launch::Subst::Error(e)) => return self.task_failed(after, e),
        };
        self.dbg.log(&format!("> Executing task: {} <", label), 2);
        if matches!(after, After::Launch(_)) {
            self.dbg.state = DbgState::Task;
        }
        self.dbg.tasks = Some(TaskRun {
            label: label.to_string(),
            stages,
            running: Vec::new(),
            after,
        });
        self.next_stage();
    }

    fn task_failed(&mut self, after: After, msg: String) {
        self.dbg.tasks = None;
        match after {
            After::Launch(plan) => {
                self.dbg.state = DbgState::Idle;
                self.dbg.pending = Some(*plan);
                self.dbg.log(&msg, 1);
                self.error(format!("{} (:debug anyway launches all the same)", msg));
            }
            After::Nothing => self.dbg.log(&msg, 1),
        }
    }

    /// Starts the next stage's tasks; with none left, what comes after.
    fn next_stage(&mut self) {
        let Some(run) = self.dbg.tasks.as_mut() else { return };
        if run.stages.is_empty() {
            let Some(run) = self.dbg.tasks.take() else { return };
            if let After::Launch(plan) = run.after {
                self.dbg.state = DbgState::Idle;
                self.go_plan(*plan);
            }
            return;
        }
        let stage = run.stages.remove(0);
        for t in stage {
            let running = self.start_task(t);
            if let Some(run) = self.dbg.tasks.as_mut() {
                run.running.push(running);
            }
        }
    }

    fn start_task(&mut self, t: Task) -> Running {
        let mut r = Running {
            active: t.background.as_ref().is_some_and(|b| b.0),
            pid: None,
            started: Instant::now(),
            task: t,
        };
        if r.task.empty {
            return r;
        }
        // Already running: a background one is reused as it is.
        if let Some(s) = self.terms.iter().find(|s| s.title == r.task.label && s.alive()) {
            r.pid = Some(s.pid());
            if r.task.is_background {
                r.task.background = None;
                r.task.is_background = false;
                r.task.empty = true;
            }
            return r;
        }
        let env: Vec<(String, Option<String>)> = r.task.env.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect();
        match self.term_run(&r.task.argv, &r.task.cwd, &env, &r.task.label) {
            Ok(pid) => {
                r.pid = Some(pid);
                if r.task.background.is_some() {
                    if let Some(s) = self.terms.iter_mut().find(|s| s.pid() == pid) {
                        s.tap = Some(String::new());
                    }
                }
            }
            Err(e) => {
                self.dbg.log(&format!("task {}: {}", r.task.label, e), 1);
                r.pid = None;
            }
        }
        r
    }

    /// Follows the running tasks. True when something changed.
    pub(crate) fn task_tick(&mut self) -> bool {
        let Some(run) = self.dbg.tasks.as_mut() else {
            return false;
        };
        if run.running.is_empty() {
            return false;
        }
        let mut failure = None;
        let mut i = 0;
        while i < run.running.len() {
            let r = &mut run.running[i];
            if r.task.empty {
                run.running.remove(i);
                continue;
            }
            let Some(term) = r.pid.and_then(|pid| self.terms.iter_mut().find(|s| s.pid() == pid)) else {
                failure = Some(format!("The task '{}' could not run.", r.task.label));
                break;
            };
            term.poll();
            if let Some(code) = term.exit {
                if code == 0 {
                    run.running.remove(i);
                    continue;
                }
                failure = Some(format!(
                    "The preLaunchTask '{}' terminated with exit code {}.",
                    r.task.label, code
                ));
                break;
            }
            if r.task.is_background {
                match &r.task.background {
                    Some(bg) => {
                        let out = term.tap.take().unwrap_or_default();
                        term.tap = Some(String::new());
                        let (active, ready) = tasks::background_state(bg, &out, r.active);
                        r.active = active || ready;
                        if ready {
                            // It keeps running in its tab.
                            term.tap = None;
                            run.running.remove(i);
                            continue;
                        }
                        if !r.active && r.started.elapsed() > BACKGROUND_START {
                            failure = Some(format!(
                                "The task '{}' didn't show its begin pattern within 5 s (problemMatcher background.beginsPattern).",
                                r.task.label
                            ));
                            break;
                        }
                    }
                    None if r.started.elapsed() > BACKGROUND_START => {
                        failure = Some(format!(
                            "The task '{}' has not exited and doesn't have a 'problemMatcher' defined (with a background pattern) that tells when it's ready.",
                            r.task.label
                        ));
                        break;
                    }
                    None => {}
                }
            }
            i += 1;
        }
        if let Some(msg) = failure {
            let after = self.dbg.tasks.take().map_or(After::Nothing, |r| r.after);
            self.task_failed(after, msg);
            return true;
        }
        let done = self.dbg.tasks.as_ref().is_some_and(|r| r.running.is_empty());
        if done {
            if let Some(label) = self.dbg.tasks.as_ref().map(|r| r.label.clone()) {
                let left = self.dbg.tasks.as_ref().is_some_and(|r| !r.stages.is_empty());
                if !left {
                    self.dbg.log(&format!("task {}: done", label), 2);
                }
            }
            self.next_stage();
            return true;
        }
        false
    }

    /// `serverReadyAction` against a piece of output (once per session).
    pub(crate) fn server_ready_check(&mut self, text: &str) {
        let Some(url) = self.dbg.ready.as_ref().and_then(|r| r.url(text)) else {
            return;
        };
        let action = self.dbg.ready.take().map(|r| r.action).unwrap_or_default();
        match action.as_str() {
            "startDebugging" => self.dbg.log(
                &format!(
                    "serverReadyAction: {} is ready (startDebugging isn't supported in ren)",
                    url
                ),
                2,
            ),
            _ => match open_url(&url) {
                Ok(()) => self.dbg.log(&format!("serverReadyAction: opened {}", url), 2),
                Err(e) => self
                    .dbg
                    .log(&format!("serverReadyAction: could not open {}: {}", url, e), 1),
            },
        }
    }

    /// The program's terminal (from `runInTerminal`), for `serverReadyAction`.
    pub(crate) fn server_ready_tick(&mut self) -> bool {
        if self.dbg.ready.is_none() {
            return false;
        }
        let Some(pid) = self.dbg.debug_term else { return false };
        let Some(t) = self.terms.iter_mut().find(|t| t.pid() == pid) else {
            return false;
        };
        let out = t.tap.as_mut().map(std::mem::take).unwrap_or_default();
        if out.is_empty() {
            return false;
        }
        self.server_ready_check(&out);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn server_ready_urls() {
        let r = ServerReady::new(&json!({})).unwrap();
        assert_eq!(r.url(" * Running on http://127.0.0.1:5000"), None);
        assert_eq!(
            r.url("Listening on port 8000").as_deref(),
            Some("http://localhost:8000")
        );
        let r = ServerReady::new(&json!({ "pattern": "Running on (https?://\\S+)" })).unwrap();
        assert_eq!(
            r.url(" * Running on http://127.0.0.1:5000").as_deref(),
            Some("http://127.0.0.1:5000")
        );
        let r =
            ServerReady::new(&json!({ "pattern": "port ([0-9]+)", "uriFormat": "https://localhost:%s/docs" })).unwrap();
        assert_eq!(r.url("on port 443").as_deref(), Some("https://localhost:443/docs"));
    }
}
