//! Containers tab: lists containers from a Docker-compatible socket
//! (`containers.socket` in settings → `DOCKER_HOST`), start/stop/restart, and
//! live logs. Everything runs the `docker` CLI on background threads.

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

const REFRESH_EVERY: Duration = Duration::from_secs(4);
const MAX_LOG: usize = 3000;

#[derive(Clone, Debug, PartialEq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: String,
}

/// How a container's status is shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Health {
    Running,
    Busy,
    Stopped,
    Bad,
}

impl Container {
    pub fn health(&self) -> Health {
        let unhealthy = self.status.contains("(unhealthy)");
        match self.state.as_str() {
            "running" if unhealthy => Health::Bad,
            "running" => Health::Running,
            "paused" | "restarting" | "removing" => Health::Busy,
            "dead" => Health::Bad,
            _ => Health::Stopped,
        }
    }
}

/// (command line, extra environment, terminal tab title)
pub type ShellCmd = (Vec<String>, Vec<(String, String)>, String);

enum Msg {
    List(Result<Vec<Container>, String>),
    Done(String, bool),
    Log(String, String),
}

pub struct Containers {
    pub list: Vec<Container>,
    pub sel: usize,
    pub scroll: usize,
    pub error: Option<String>,
    pub note: Option<(String, bool)>,
    pub logs: Vec<String>,
    pub log_id: Option<String>,
    /// Lines scrolled up from the end of the log.
    pub log_back: usize,
    pub loaded: bool,
    pub socket: String,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    log_child: Option<Child>,
    last: Option<Instant>,
    refreshing: bool,
}

/// `docker` with `DOCKER_HOST` set when a socket is configured.
pub fn docker(socket: &str) -> Command {
    let mut c = Command::new("docker");
    if !socket.is_empty() {
        let host = if socket.contains("://") {
            socket.to_string()
        } else {
            format!("unix://{}", socket)
        };
        c.env("DOCKER_HOST", host);
    }
    c
}

pub fn parse_ps(out: &str) -> Vec<Container> {
    out.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .map(|v| {
            let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
            Container {
                id: s("ID"),
                name: s("Names"),
                image: s("Image"),
                state: s("State").to_lowercase(),
                status: s("Status"),
                ports: s("Ports"),
            }
        })
        .collect()
}

impl Default for Containers {
    fn default() -> Self {
        Self::new()
    }
}

impl Containers {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Containers {
            list: Vec::new(),
            sel: 0,
            scroll: 0,
            error: None,
            note: None,
            logs: Vec::new(),
            log_id: None,
            log_back: 0,
            loaded: false,
            socket: String::new(),
            tx,
            rx,
            log_child: None,
            last: None,
            refreshing: false,
        }
    }

    pub fn refresh(&mut self) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        self.last = Some(Instant::now());
        let tx = self.tx.clone();
        let socket = self.socket.clone();
        thread::spawn(move || {
            let r = docker(&socket)
                .args(["ps", "-a", "--format", "{{json .}}"])
                .stdin(Stdio::null())
                .output();
            let msg = match r {
                Ok(o) if o.status.success() => Ok(parse_ps(&String::from_utf8_lossy(&o.stdout))),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                Err(e) => Err(format!("docker: {}", e)),
            };
            let _ = tx.send(Msg::List(msg));
        });
    }

    pub fn selected(&self) -> Option<&Container> {
        self.list.get(self.sel)
    }

    /// `docker start|stop|restart <id>` in the background, then refresh.
    pub fn action(&mut self, verb: &'static str) {
        let Some(c) = self.selected().cloned() else { return };
        self.note = Some((format!("{} {}…", verb, c.name), false));
        let tx = self.tx.clone();
        let socket = self.socket.clone();
        thread::spawn(move || {
            let r = docker(&socket).args([verb, &c.id]).stdin(Stdio::null()).output();
            let (m, err) = match r {
                Ok(o) if o.status.success() => (format!("{} {}: done", verb, c.name), false),
                Ok(o) => (String::from_utf8_lossy(&o.stderr).trim().to_string(), true),
                Err(e) => (format!("docker: {}", e), true),
            };
            let _ = tx.send(Msg::Done(m, err));
        });
    }

    /// Follows `docker logs -f` for the selected container.
    pub fn follow_logs(&mut self) {
        let Some(c) = self.selected().cloned() else { return };
        if self.log_id.as_deref() == Some(&c.id) && self.log_child.is_some() {
            return;
        }
        self.stop_logs();
        self.logs.clear();
        self.log_back = 0;
        self.log_id = Some(c.id.clone());
        let child = docker(&self.socket)
            .args(["logs", "--tail", "500", "-f", &c.id])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        match child {
            Ok(mut ch) => {
                for out in [
                    ch.stdout.take().map(|o| Box::new(o) as Box<dyn std::io::Read + Send>),
                    ch.stderr.take().map(|o| Box::new(o) as Box<dyn std::io::Read + Send>),
                ]
                .into_iter()
                .flatten()
                {
                    let tx = self.tx.clone();
                    let id = c.id.clone();
                    thread::spawn(move || {
                        for line in BufReader::new(out).lines() {
                            let Ok(l) = line else { break };
                            if tx.send(Msg::Log(id.clone(), l)).is_err() {
                                break;
                            }
                        }
                    });
                }
                self.log_child = Some(ch);
            }
            Err(e) => self.logs.push(format!("docker logs: {}", e)),
        }
    }

    pub fn stop_logs(&mut self) {
        if let Some(mut c) = self.log_child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Applies background results; refreshes periodically while `visible`.
    pub fn tick(&mut self, visible: bool) -> bool {
        let mut changed = false;
        while let Ok(m) = self.rx.try_recv() {
            changed = true;
            match m {
                Msg::List(r) => {
                    self.refreshing = false;
                    self.loaded = true;
                    match r {
                        Ok(list) => {
                            let keep = self.selected().map(|c| c.id.clone());
                            self.list = list;
                            self.error = None;
                            if let Some(id) = keep {
                                if let Some(i) = self.list.iter().position(|c| c.id == id) {
                                    self.sel = i;
                                }
                            }
                            self.sel = self.sel.min(self.list.len().saturating_sub(1));
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Msg::Done(m, err) => {
                    self.note = Some((m, err));
                    self.refreshing = false;
                    self.refresh();
                }
                Msg::Log(id, l) => {
                    if self.log_id.as_deref() == Some(&id) {
                        self.logs.push(l);
                        if self.logs.len() > MAX_LOG {
                            self.logs.drain(..self.logs.len() - MAX_LOG);
                        }
                        if self.log_back > 0 {
                            self.log_back += 1;
                        }
                    }
                }
            }
        }
        if visible && self.last.is_none_or(|t| t.elapsed() >= REFRESH_EVERY) {
            self.refresh();
        }
        changed
    }

    /// The command line that opens a shell inside the selected container.
    pub fn shell_cmd(&self) -> Option<ShellCmd> {
        let c = self.selected()?;
        let cmd = vec![
            "docker".into(),
            "exec".into(),
            "-it".into(),
            c.id.clone(),
            "sh".into(),
            "-c".into(),
            "command -v bash >/dev/null 2>&1 && exec bash || exec sh".into(),
        ];
        let mut env = Vec::new();
        if !self.socket.is_empty() {
            let host = if self.socket.contains("://") {
                self.socket.clone()
            } else {
                format!("unix://{}", self.socket)
            };
            env.push(("DOCKER_HOST".into(), host));
        }
        Some((cmd, env, c.name.clone()))
    }
}

impl Drop for Containers {
    fn drop(&mut self) {
        self.stop_logs();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_docker_ps_json() {
        let out = r#"{"ID":"abc","Names":"web","Image":"nginx","State":"running","Status":"Up 2 hours (healthy)","Ports":"0.0.0.0:80->80/tcp"}
{"ID":"def","Names":"db","Image":"postgres","State":"exited","Status":"Exited (0) 1 day ago","Ports":""}
{"ID":"ghi","Names":"api","Image":"x","State":"running","Status":"Up 1 min (unhealthy)","Ports":""}"#;
        let l = parse_ps(out);
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].name, "web");
        assert_eq!(l[0].health(), Health::Running);
        assert_eq!(l[1].health(), Health::Stopped);
        assert_eq!(l[2].health(), Health::Bad);
    }
}
