use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub fn path() -> Option<PathBuf> {
    Some(crate::settings::dir()?.join("state"))
}

pub fn load() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    let Some(p) = path() else { return m };
    let Ok(text) = fs::read_to_string(p) else { return m };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            m.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    m
}

pub fn set(key: &str, value: &str) -> std::io::Result<()> {
    let p = path().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no config directory"))?;
    let mut m = load();
    m.insert(key.to_string(), value.to_string());
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    let mut out = String::from("# ren state (managed by ren; settings live in settings.json)\n");
    for (k, v) in m {
        out += &format!("{} = {}\n", k, v);
    }
    fs::write(p, out)
}

pub fn get_list(key: &str) -> Vec<String> {
    load()
        .get(key)
        .map(|v| v.split('|').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect())
        .unwrap_or_default()
}
