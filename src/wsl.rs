// fyi - for your information
// Copyright (C) 2026 S.G.Johansson <s.johansson.it@gmail.com>
// https://voidflow.tech/
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// A copy is included in the LICENSE file, or see
// http://www.apache.org/licenses/LICENSE-2.0
//
//! WSL awareness: Windows path → Linux path conversion (and back for display).
//!
//! Handles `C:\x`, `C:/x`, `C:`, `\\wsl.localhost\<distro>\x`, `\\wsl$\<distro>\x`
//! and the shell-mangled form of an unquoted `C:\Users\me`, which bash hands
//! over as `C:Usersme`: separators are recovered by matching the run of text
//! against the names that actually exist on disk.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_CANDIDATES: usize = 16;
const MAX_SPLIT_DEPTH: usize = 32;

#[derive(Debug, Clone)]
pub struct Wsl {
    /// (drive letter uppercase, mount point), longest mount point first.
    pub drives: Vec<(char, PathBuf)>,
    pub distro: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Resolve {
    /// Resolved path; `win` holds the canonical Windows spelling when converted.
    Found {
        path: PathBuf,
        win: Option<String>,
    },
    Ambiguous(Vec<PathBuf>),
    NotFound(String),
    /// Not Windows-shaped, or not on WSL: caller treats it as a plain path.
    NotWindows,
}

impl Wsl {
    pub fn detect() -> Option<Wsl> {
        let env = std::env::var_os("WSL_DISTRO_NAME").is_some();
        let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false);
        if !env && !kernel {
            return None;
        }
        let drives = fs::read_to_string("/proc/self/mountinfo")
            .map(|s| parse_mountinfo(&s))
            .unwrap_or_default();
        Some(Wsl {
            drives,
            distro: std::env::var("WSL_DISTRO_NAME").ok(),
        })
    }

    pub fn mount_for(&self, letter: char) -> PathBuf {
        let up = letter.to_ascii_uppercase();
        self.drives
            .iter()
            .find(|(l, _)| *l == up)
            .map(|(_, p)| p.clone())
            .unwrap_or_else(|| PathBuf::from(format!("/mnt/{}", letter.to_ascii_lowercase())))
    }

    /// `/mnt/c/Users/me` → `C:\Users\me`.
    pub fn to_windows(&self, p: &Path) -> Option<String> {
        let mut candidates: Vec<(char, PathBuf)> = self.drives.clone();
        if candidates.is_empty() {
            // No mountinfo (tests, odd setups): assume the default automount root.
            if let Ok(rest) = p.strip_prefix("/mnt") {
                let first = rest.components().next()?.as_os_str().to_str()?;
                let c = first.chars().next()?;
                if first.len() == 1 && c.is_ascii_alphabetic() {
                    candidates.push((
                        c.to_ascii_uppercase(),
                        PathBuf::from(format!("/mnt/{first}")),
                    ));
                }
            }
        }
        for (l, m) in &candidates {
            if let Ok(rest) = p.strip_prefix(m) {
                let parts: Vec<String> = rest
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                return Some(format!("{l}:\\{}", parts.join("\\")));
            }
        }
        None
    }

    pub fn resolve(&self, arg: &str) -> Resolve {
        if let Some((letter, rest)) = split_drive(arg) {
            let base = self.mount_for(letter);
            let segs: Vec<&str> = rest.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
            let direct = segs.iter().fold(base.clone(), |p, s| p.join(s));
            if direct.exists() {
                let win = self.to_windows(&direct);
                return Resolve::Found { path: direct, win };
            }
            let mut found = fuzzy_resolve(&base, &segs);
            return match found.len() {
                0 => Resolve::NotFound(format!("{arg}: no such path under {}", base.display())),
                1 => {
                    let path = found.remove(0);
                    let win = self.to_windows(&path);
                    Resolve::Found { path, win }
                }
                _ => Resolve::Ambiguous(found),
            };
        }
        if let Some((host, rest)) = split_unc(arg) {
            let is_wsl_host =
                host.eq_ignore_ascii_case("wsl.localhost") || host.eq_ignore_ascii_case("wsl$");
            if !is_wsl_host {
                return Resolve::NotFound(format!("{arg}: network shares must be mounted first"));
            }
            let mut it = rest.splitn(2, '\\');
            let distro = it.next().unwrap_or("");
            let tail = it.next().unwrap_or("");
            match &self.distro {
                Some(d) if d.eq_ignore_ascii_case(distro) => {
                    let path = PathBuf::from(format!("/{}", tail.replace('\\', "/")));
                    return Resolve::Found {
                        path,
                        win: Some(arg.replace('/', "\\")),
                    };
                }
                _ => {
                    return Resolve::NotFound(format!(
                        "{arg}: distro '{distro}' is not this one ({})",
                        self.distro.as_deref().unwrap_or("unknown")
                    ));
                }
            }
        }
        Resolve::NotWindows
    }
}

/// `C:\x`, `c:/x`, `C:`, `C:Usersme` → ('C', rest).
pub fn split_drive(arg: &str) -> Option<(char, &str)> {
    let b = arg.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        Some((b[0] as char, &arg[2..]))
    } else {
        None
    }
}

/// `\\host\rest` or `//host/rest` → (host, rest with backslashes).
fn split_unc(arg: &str) -> Option<(String, String)> {
    let norm = arg.replace('/', "\\");
    let body = norm.strip_prefix("\\\\")?;
    let mut it = body.splitn(2, '\\');
    let host = it.next()?.to_string();
    if host.is_empty() {
        return None;
    }
    Some((host, it.next().unwrap_or("").to_string()))
}

pub fn looks_windows(arg: &str) -> bool {
    split_drive(arg).is_some() || split_unc(arg).is_some()
}

/// Resolve each segment against the real tree; a segment that does not exist
/// verbatim is split into existing names (case-insensitive, ASCII).
pub fn fuzzy_resolve(base: &Path, segs: &[&str]) -> Vec<PathBuf> {
    let mut cur = vec![base.to_path_buf()];
    for seg in segs {
        let mut next = Vec::new();
        for d in &cur {
            let exact = d.join(seg);
            if exact.exists() {
                next.push(exact);
            } else {
                split_match(d, seg, 0, &mut next);
            }
            if next.len() >= MAX_CANDIDATES {
                break;
            }
        }
        next.sort();
        next.dedup();
        if next.is_empty() {
            return next;
        }
        cur = next;
    }
    cur
}

fn split_match(dir: &Path, s: &str, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_SPLIT_DEPTH || out.len() >= MAX_CANDIDATES {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut names: Vec<String> = rd
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    // Longest names first: prefer "Program Files" over "Program".
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for n in names {
        if n.is_empty() || n.len() > s.len() || !s.is_char_boundary(n.len()) {
            continue;
        }
        if !s[..n.len()].eq_ignore_ascii_case(&n) {
            continue;
        }
        let rest = &s[n.len()..];
        let p = dir.join(&n);
        if rest.is_empty() {
            out.push(p);
        } else if p.is_dir() {
            split_match(&p, rest, depth + 1, out);
        }
        if out.len() >= MAX_CANDIDATES {
            return;
        }
    }
}

/// Extract drive mounts from /proc/self/mountinfo (drvfs, or 9p with aname=drvfs).
pub fn parse_mountinfo(s: &str) -> Vec<(char, PathBuf)> {
    let mut out = Vec::new();
    for line in s.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let lf: Vec<&str> = left.split(' ').collect();
        let rf: Vec<&str> = right.split(' ').collect();
        if lf.len() < 5 || rf.len() < 2 {
            continue;
        }
        let mount = unescape(lf[4]);
        let fstype = rf[0];
        let source = unescape(rf[1]);
        let opts = rf.get(2).copied().unwrap_or("");
        let drvfs = fstype == "drvfs" || opts.contains("aname=drvfs");
        if !drvfs {
            continue;
        }
        let letter = split_drive(&source).map(|(l, _)| l).or_else(|| {
            let i = opts.find("path=")?;
            split_drive(&opts[i + 5..]).map(|(l, _)| l)
        });
        if let Some(l) = letter {
            out.push((l.to_ascii_uppercase(), PathBuf::from(mount)));
        }
    }
    out.sort_by_key(|(_, p)| std::cmp::Reverse(p.as_os_str().len()));
    out
}

/// mountinfo octal escapes: `\040` → space, `\134` → backslash.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 4 <= b.len() {
            let oct = &b[i + 1..i + 4];
            if oct.iter().all(|c| (b'0'..=b'7').contains(c)) {
                let v = oct.iter().fold(0u32, |acc, c| acc * 8 + (c - b'0') as u32);
                if v <= 0xff {
                    out.push(v as u8);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOUNTINFO: &str = "\
1 0 8:48 / / rw,relatime - ext4 /dev/sdc rw\n\
95 1 0:59 / /mnt/c rw,noatime - 9p C:\\134 rw,dirsync,aname=drvfs;path=C:\\;uid=1000\n\
96 1 0:60 / /mnt/my\\040drive rw,noatime - 9p D:\\134 rw,aname=drvfs;path=D:\\\n\
97 1 0:61 / /mnt/wslg rw - tmpfs none rw\n";

    #[test]
    fn mountinfo() {
        let m = parse_mountinfo(MOUNTINFO);
        assert!(m.contains(&('C', PathBuf::from("/mnt/c"))));
        assert!(m.contains(&('D', PathBuf::from("/mnt/my drive"))));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn drive_split() {
        assert_eq!(split_drive("C:\\Users"), Some(('C', "\\Users")));
        assert_eq!(split_drive("c:"), Some(('c', "")));
        assert_eq!(split_drive("C:Userslunal"), Some(('C', "Userslunal")));
        assert_eq!(split_drive("/home"), None);
        assert!(looks_windows("\\\\wsl.localhost\\Ubuntu\\home"));
        assert!(looks_windows("//wsl$/Ubuntu/home"));
    }

    #[test]
    fn to_windows() {
        let w = Wsl {
            drives: vec![('C', "/mnt/c".into())],
            distro: None,
        };
        assert_eq!(
            w.to_windows(Path::new("/mnt/c/Users/me")).as_deref(),
            Some("C:\\Users\\me")
        );
        assert_eq!(w.to_windows(Path::new("/mnt/c")).as_deref(), Some("C:\\"));
        assert_eq!(w.to_windows(Path::new("/home")), None);
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fyi-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn mangled_path_recovery() {
        let root = tmp("mangle");
        fs::create_dir_all(root.join("Users/lunal/Documents")).unwrap();
        fs::create_dir_all(root.join("Program Files/Git")).unwrap();
        fs::create_dir_all(root.join("Program")).unwrap();
        let w = Wsl {
            drives: vec![('C', root.clone())],
            distro: None,
        };

        // bash turns C:\Users\lunal into C:Userslunal
        match w.resolve("C:Userslunal") {
            Resolve::Found { path, win } => {
                assert_eq!(path, root.join("Users/lunal"));
                assert_eq!(win.as_deref(), Some("C:\\Users\\lunal"));
            }
            r => panic!("{r:?}"),
        }
        match w.resolve("c:userslunaldocuments") {
            Resolve::Found { path, .. } => assert_eq!(path, root.join("Users/lunal/Documents")),
            r => panic!("{r:?}"),
        }
        // quoted / forward slashes resolve directly
        match w.resolve("C:/Users/lunal") {
            Resolve::Found { path, .. } => assert_eq!(path, root.join("Users/lunal")),
            r => panic!("{r:?}"),
        }
        // space-joined args (C:\Program Files\Git unquoted → "C:Program" "FilesGit")
        match w.resolve("C:Program FilesGit") {
            Resolve::Found { path, .. } => assert_eq!(path, root.join("Program Files/Git")),
            r => panic!("{r:?}"),
        }
        assert!(matches!(w.resolve("C:Nope"), Resolve::NotFound(_)));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ambiguity_is_reported() {
        let root = tmp("ambig");
        fs::create_dir_all(root.join("ab/c")).unwrap();
        fs::create_dir_all(root.join("a/bc")).unwrap();
        let w = Wsl {
            drives: vec![('C', root.clone())],
            distro: None,
        };
        match w.resolve("C:abc") {
            Resolve::Ambiguous(v) => assert_eq!(v.len(), 2),
            r => panic!("{r:?}"),
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unc() {
        let w = Wsl {
            drives: vec![],
            distro: Some("Ubuntu".into()),
        };
        match w.resolve("\\\\wsl.localhost\\Ubuntu\\home\\me") {
            Resolve::Found { path, .. } => assert_eq!(path, PathBuf::from("/home/me")),
            r => panic!("{r:?}"),
        }
        assert!(matches!(
            w.resolve("//wsl$/Debian/home"),
            Resolve::NotFound(_)
        ));
        assert!(matches!(
            w.resolve("\\\\server\\share"),
            Resolve::NotFound(_)
        ));
        assert_eq!(w.resolve("/home"), Resolve::NotWindows);
    }
}
