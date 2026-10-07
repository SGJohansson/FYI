//! Filesystem scanning into a serialisable tree of `Node`s.
//!
//! The same model is rendered live and stored in history, so a replay renders
//! exactly what was seen, re-laid-out for the current terminal.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    Other,
}

/// Effective access for the invoking user, collapsed into the colour classes.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    Default,
    NoWrite,
    Exec,
    ExecWrite,
    NoRead,
    DirReadOnly,
    DirFull,
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Node {
    #[serde(rename = "n")]
    pub name: String,
    #[serde(rename = "k")]
    pub kind: Kind,
    /// Resolves to a directory (symlinks followed).
    #[serde(rename = "d", default, skip_serializing_if = "is_false")]
    pub is_dir: bool,
    #[serde(rename = "s", default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(rename = "a")]
    pub access: Access,
    #[serde(rename = "l", default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(rename = "b", default, skip_serializing_if = "is_false")]
    pub broken: bool,
    #[serde(rename = "u", default, skip_serializing_if = "is_false")]
    pub setuid: bool,
    #[serde(rename = "g", default, skip_serializing_if = "is_false")]
    pub setgid: bool,
    /// `Some` when this directory was expanded.
    #[serde(rename = "c", default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<Node>>,
    /// Entries not included (per-directory limit or history pruning).
    #[serde(rename = "od", default, skip_serializing_if = "is_zero")]
    pub omitted_dirs: u64,
    #[serde(rename = "of", default, skip_serializing_if = "is_zero")]
    pub omitted_files: u64,
    #[serde(rename = "e", default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Node {
    fn new(name: String, kind: Kind) -> Self {
        Node {
            name,
            kind,
            is_dir: false,
            size: None,
            access: Access::Default,
            link: None,
            broken: false,
            setuid: false,
            setgid: false,
            children: None,
            omitted_dirs: 0,
            omitted_files: 0,
            error: None,
        }
    }

    /// (dirs, files) including omitted entries.
    pub fn counts(&self) -> (u64, u64) {
        let (mut d, mut f) = (self.omitted_dirs, self.omitted_files);
        for c in self.children.iter().flatten() {
            if c.is_dir {
                d += 1;
            } else {
                f += 1;
            }
        }
        (d, f)
    }

    pub fn omitted(&self) -> u64 {
        self.omitted_dirs + self.omitted_files
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    /// Flat listings: directories first.
    DirsFirst,
    /// Trees: a directory's own files first, then its subtrees.
    FilesFirst,
}

#[derive(Clone, Copy, Debug)]
pub struct ScanOpts {
    pub all: bool,
    /// Levels of children to read below the root (1 = flat listing).
    pub depth: usize,
    /// Max entries kept per directory (None = unlimited).
    pub limit: Option<usize>,
    pub dir_sizes: bool,
    pub order: Order,
}

/// Replace control characters so file names cannot inject terminal escapes.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

fn lossy(os: &std::ffi::OsStr) -> String {
    sanitize(&os.to_string_lossy())
}

pub fn short_err(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied => "permission denied".into(),
        io::ErrorKind::NotFound => "not found".into(),
        _ => {
            let s = e.to_string();
            match s.find(" (os error") {
                Some(i) => s[..i].to_lowercase(),
                None => s,
            }
        }
    }
}

fn can(path: &Path, mode: libc::c_int) -> bool {
    let Ok(c) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: valid NUL-terminated path, AT_FDCWD, no out-pointers.
    unsafe { libc::faccessat(libc::AT_FDCWD, c.as_ptr(), mode, libc::AT_EACCESS) == 0 }
}

pub fn classify(is_dir: bool, r: bool, w: bool, x: bool) -> Access {
    if is_dir {
        if !r || !x {
            Access::NoRead
        } else if !w {
            Access::DirReadOnly
        } else {
            Access::DirFull
        }
    } else if !r {
        Access::NoRead
    } else if x && w {
        Access::ExecWrite
    } else if x {
        Access::Exec
    } else if !w {
        Access::NoWrite
    } else {
        Access::Default
    }
}

/// lstat + access check for one entry, without descending.
pub fn stat_node(path: &Path, name: String) -> Node {
    let lmeta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            let mut n = Node::new(name, Kind::Other);
            n.access = Access::NoRead;
            n.error = Some(short_err(&e));
            return n;
        }
    };
    let ft = lmeta.file_type();
    let mut node;
    let meta = if ft.is_symlink() {
        node = Node::new(name, Kind::Symlink);
        node.link = fs::read_link(path).ok().map(|t| lossy(t.as_os_str()));
        match fs::metadata(path) {
            Ok(m) => m,
            Err(_) => {
                node.broken = true;
                node.access = Access::NoRead;
                return node;
            }
        }
    } else {
        let kind = if ft.is_dir() {
            Kind::Dir
        } else if ft.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        node = Node::new(name, kind);
        lmeta
    };
    node.is_dir = meta.is_dir();
    if !node.is_dir {
        node.size = Some(meta.len());
        let mode = meta.mode();
        node.setuid = mode & 0o4000 != 0;
        node.setgid = mode & 0o2000 != 0;
    }
    node.access = classify(
        node.is_dir,
        can(path, libc::R_OK),
        can(path, libc::W_OK),
        can(path, libc::X_OK),
    );
    node
}

/// Scan `path` as the root of a listing. A symlinked root directory is followed.
pub fn scan_root(path: &Path, display: String, opts: &ScanOpts) -> Node {
    let mut node = stat_node(path, display);
    if node.is_dir && opts.depth > 0 {
        expand(&mut node, path, opts, opts.depth);
    }
    node
}

fn expand(node: &mut Node, path: &Path, opts: &ScanOpts, depth_left: usize) {
    let rd = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(e) => {
            node.error = Some(short_err(&e));
            return;
        }
    };
    let mut kids: Vec<Node> = rd
        .filter_map(Result::ok)
        .filter(|e| opts.all || !e.file_name().as_bytes().starts_with(b"."))
        .map(|e| stat_node(&e.path(), lossy(&e.file_name())))
        .collect();
    sort_nodes(&mut kids, opts.order);

    if let Some(limit) = opts.limit
        && kids.len() > limit
    {
        for k in kids.drain(limit..) {
            if k.is_dir {
                node.omitted_dirs += 1;
            } else {
                node.omitted_files += 1;
            }
        }
    }

    if depth_left > 1 {
        for k in kids.iter_mut() {
            // Never descend through symlinks below the root: avoids cycles.
            if k.is_dir && k.kind == Kind::Dir {
                let p = path.join(&k.name);
                expand(k, &p, opts, depth_left - 1);
            }
        }
    }

    if opts.dir_sizes {
        for k in kids.iter_mut() {
            if k.is_dir && k.kind == Kind::Dir && k.size.is_none() {
                k.size = Some(dir_size(&path.join(&k.name)));
            }
        }
    }
    node.children = Some(kids);

    if opts.dir_sizes && node.size.is_none() {
        // Sum is only exact when nothing was hidden or omitted.
        let complete = opts.all
            && node.omitted() == 0
            && node
                .children
                .iter()
                .flatten()
                .all(|c| !c.is_dir || c.kind != Kind::Dir || c.size.is_some());
        node.size = Some(if complete {
            node.children
                .iter()
                .flatten()
                .filter(|c| c.kind != Kind::Symlink)
                .filter_map(|c| c.size)
                .sum()
        } else {
            dir_size(path)
        });
    }
}

/// Apparent size of everything under `path`. Symlinks are not followed and
/// count as zero; unreadable subtrees are skipped.
pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(rd) = fs::read_dir(&p) else { continue };
        for e in rd.filter_map(Result::ok) {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if !ft.is_symlink()
                && let Ok(m) = e.metadata()
            {
                total += m.len();
            }
        }
    }
    total
}

pub fn sort_nodes(v: &mut [Node], order: Order) {
    v.sort_by(|a, b| {
        let group = match order {
            Order::DirsFirst => b.is_dir.cmp(&a.is_dir),
            Order::FilesFirst => a.is_dir.cmp(&b.is_dir),
        };
        group.then_with(|| natural_cmp(&a.name, &b.name))
    });
}

/// Case-insensitive natural order ("file2" < "file10"); leading dots ignored.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let ka = a.trim_start_matches('.');
    let kb = b.trim_start_matches('.');
    let (mut ia, mut ib) = (ka.chars().peekable(), kb.chars().peekable());
    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ia.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ia.next();
                }
                let mut nb = String::new();
                while let Some(c) = ib.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    ib.next();
                }
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(ca), Some(cb)) => {
                let o = ca.to_lowercase().cmp(cb.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                ia.next();
                ib.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural() {
        let mut v = vec!["file10", "File2", ".hidden", "file1", "a"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["a", "file1", "File2", "file10", ".hidden"]);
    }

    #[test]
    fn classes() {
        assert_eq!(classify(false, false, true, true), Access::NoRead);
        assert_eq!(classify(false, true, true, true), Access::ExecWrite);
        assert_eq!(classify(false, true, false, true), Access::Exec);
        assert_eq!(classify(false, true, false, false), Access::NoWrite);
        assert_eq!(classify(false, true, true, false), Access::Default);
        assert_eq!(classify(true, true, true, true), Access::DirFull);
        assert_eq!(classify(true, true, false, true), Access::DirReadOnly);
        assert_eq!(classify(true, true, true, false), Access::NoRead);
    }

    #[test]
    fn sanitizes_control_chars() {
        assert_eq!(sanitize("a\x1b[31mb\n"), "a?[31mb?");
    }
}
