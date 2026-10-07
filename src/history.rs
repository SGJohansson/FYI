//! Listing history: the last N listings are stored in full (model, not ANSI)
//! so they can be replayed and re-laid-out for the current terminal.
//!
//! Layout: `$XDG_STATE_HOME/fyi/history/<unix_nanos>-<key>.json`, two lines per
//! file: a small header (cheap to read for `--hist`) and the node tree.

use crate::model::Node;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_ENTRIES: usize = 50;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Flat,
    Tree,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Head {
    pub ts: u64,
    pub mode: Mode,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win: Option<String>,
    pub entries: usize,
    #[serde(default)]
    pub pruned: bool,
}

pub struct Entry {
    pub head: Head,
    pub root: Node,
}

pub fn dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("fyi").join("history"))
}

fn key(path: &str, mode: Mode) -> String {
    // FNV-1a: stable across builds, unlike DefaultHasher.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in path.bytes().chain([mode as u8]) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Newest first.
fn files() -> Vec<PathBuf> {
    let Some(d) = dir() else { return vec![] };
    let Ok(rd) = fs::read_dir(&d) else {
        return vec![];
    };
    let mut v: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    v.sort();
    v.reverse();
    v
}

/// Keep only the first `depth` levels below `node`.
fn cut_depth(node: &mut Node, depth: usize) {
    if let Some(kids) = node.children.as_mut() {
        if depth == 0 {
            for k in kids.iter() {
                if k.is_dir {
                    node.omitted_dirs += 1;
                } else {
                    node.omitted_files += 1;
                }
            }
            node.children = Some(vec![]);
            return;
        }
        for k in kids.iter_mut() {
            if depth == 1 {
                if k.children.is_some() {
                    k.children = None;
                }
            } else {
                cut_depth(k, depth - 1);
            }
        }
    }
}

fn max_depth(node: &Node) -> usize {
    node.children
        .iter()
        .flatten()
        .map(|c| 1 + max_depth(c))
        .max()
        .unwrap_or(0)
}

/// Shrink `root` until its JSON fits `limit`: drop the deepest levels first,
/// then trim the top-level entry list.
pub fn fit(root: &mut Node, limit: usize) -> (String, bool) {
    let mut json = serde_json::to_string(root).unwrap_or_default();
    if json.len() <= limit {
        return (json, false);
    }
    let mut depth = max_depth(root);
    while json.len() > limit && depth > 1 {
        depth -= 1;
        cut_depth(root, depth);
        json = serde_json::to_string(root).unwrap_or_default();
    }
    while json.len() > limit {
        let Some(kids) = root.children.as_mut() else {
            break;
        };
        if kids.is_empty() {
            break;
        }
        let ratio = limit as f64 / json.len() as f64;
        let keep = ((kids.len() as f64 * ratio * 0.95) as usize).min(kids.len() - 1);
        for k in kids.drain(keep..) {
            if k.is_dir {
                root.omitted_dirs += 1;
            } else {
                root.omitted_files += 1;
            }
        }
        json = serde_json::to_string(root).unwrap_or_default();
    }
    (json, true)
}

pub fn record(
    mut root: Node,
    mode: Mode,
    path: &str,
    win: Option<&str>,
    limit: usize,
) -> std::io::Result<()> {
    let Some(d) = dir() else { return Ok(()) };
    fs::create_dir_all(&d)?;
    let k = key(path, mode);
    let existing = files();

    // Re-listing the same place replaces its previous entry instead of stacking.
    if let Some(newest) = existing.first()
        && newest
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.ends_with(&k))
    {
        let _ = fs::remove_file(newest);
    }

    let entries = root.children.as_ref().map(|c| c.len()).unwrap_or(0);
    let (body, pruned) = fit(&mut root, limit);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let head = Head {
        ts: ts.as_secs(),
        mode,
        path: path.to_string(),
        win: win.map(str::to_string),
        entries,
        pruned,
    };

    let name = format!("{:020}-{k}.json", ts.as_nanos());
    let tmp = d.join(format!(".{name}.tmp"));
    {
        let mut f = fs::File::create(&tmp)?;
        writeln!(f, "{}", serde_json::to_string(&head).unwrap_or_default())?;
        writeln!(f, "{body}")?;
    }
    fs::rename(&tmp, d.join(&name))?;

    for old in files().into_iter().skip(MAX_ENTRIES) {
        let _ = fs::remove_file(old);
    }
    Ok(())
}

fn read_head(p: &PathBuf) -> Option<Head> {
    let mut line = String::new();
    BufReader::new(fs::File::open(p).ok()?)
        .read_line(&mut line)
        .ok()?;
    serde_json::from_str(&line).ok()
}

/// Headers, newest first (index 0 = `-b 1`).
pub fn list() -> Vec<Head> {
    files().iter().filter_map(read_head).collect()
}

/// `n` = 1 for the most recent listing.
pub fn load(n: usize) -> Result<Entry, String> {
    let all = files();
    if all.is_empty() {
        return Err("history is empty".into());
    }
    let p = all
        .get(n.saturating_sub(1))
        .ok_or_else(|| format!("history has {} entries (asked for #{n})", all.len()))?;
    let text = fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut lines = text.lines();
    let head: Head = serde_json::from_str(lines.next().unwrap_or(""))
        .map_err(|e| format!("corrupt history entry: {e}"))?;
    let root: Node = serde_json::from_str(lines.next().unwrap_or(""))
        .map_err(|e| format!("corrupt history entry: {e}"))?;
    Ok(Entry { head, root })
}

/// Local time `YYYY-MM-DD HH:MM`.
pub fn fmt_time(ts: u64) -> String {
    let t = ts as libc::time_t;
    // SAFETY: zeroed tm is a valid out-buffer for localtime_r.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&t, &mut tm).is_null() };
    if !ok {
        return ts.to_string();
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Order, ScanOpts, scan_root};

    #[test]
    fn fit_shrinks_under_limit() {
        let root = std::env::temp_dir().join(format!("fyi-hist-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for i in 0..40 {
            fs::create_dir_all(root.join(format!("d{i}/sub/deeper"))).unwrap();
            fs::write(
                root.join(format!("d{i}/sub/deeper/file_with_a_long_name_{i}.txt")),
                "x",
            )
            .unwrap();
            fs::write(root.join(format!("f{i}.txt")), "y").unwrap();
        }
        let opts = ScanOpts {
            all: false,
            depth: 10,
            limit: None,
            dir_sizes: false,
            order: Order::FilesFirst,
        };
        let mut n = scan_root(&root, "x".into(), &opts);
        let full = serde_json::to_string(&n).unwrap().len();
        let (json, pruned) = fit(&mut n, full / 4);
        assert!(pruned);
        assert!(json.len() <= full / 4, "{} > {}", json.len(), full / 4);
        let (d, f) = n.counts();
        assert_eq!(d + f, 80, "omitted entries must still be counted");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn key_is_stable() {
        assert_eq!(key("/a", Mode::Flat), key("/a", Mode::Flat));
        assert_ne!(key("/a", Mode::Flat), key("/a", Mode::Tree));
    }
}
