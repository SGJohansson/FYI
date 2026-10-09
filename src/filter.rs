// fyi - for your information
// Copyright (C) 2026 S.G.Johansson <s.johansson.it@gmail.com>
// https://voidflow.tech/
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// A copy is included in the LICENSE file, or see
// http://www.apache.org/licenses/LICENSE-2.0
//
//! `--ext` / `--eext` filtering and the `--legend` page.
//!
//! A term is a built-in keyword (`dir`, `bin`, `small`, `l1`, …) or, for any
//! other word, a file extension. No wildcards: `mp3` matches `name.mp3` only.

use crate::model::{Access, Kind, Node};
use crate::style::{
    AC_DIR_FULL, AC_DIR_RO, AC_EXEC, AC_EXEC_WRITE, AC_NO_READ, AC_NO_WRITE, BANDS, Painter, Sty,
    size_sty,
};

/// Built-in keywords. The parser and `--legend` both follow this list.
pub const KEYWORDS: &[(&str, &str)] = &[
    ("dir", "directories (incl. links to directories)"),
    ("file", "regular files"),
    ("bin", "executable files (alias: exe)"),
    ("hidden", "dotfiles; --ext=hidden implies -a"),
    ("link", "symlinks"),
    ("broken", "broken symlinks"),
    ("ro", "not writable by you"),
    ("locked", "not readable by you"),
    ("suid", "setuid / setgid"),
    ("empty", "zero-byte files"),
];

const SUGGEST: &[(&str, &str)] = &[
    ("--ext=dir,mp3,flac,ogg,opus,m4a", "music"),
    ("--ext=dir,mkv,mp4,webm,avi,mov", "video"),
    ("--ext=jpg,jpeg,png,webp,gif,heic", "images"),
    ("--ext=pdf,md,txt,odt,docx,epub", "documents"),
    ("--ext=zip,7z,rar,tar.gz,tgz,xz,zst", "archives"),
    ("--ext=sh,py,rs,c,h,js,ts,go", "source"),
    ("--ext=mkv+large", "only big videos"),
    ("-r --ext=large", "where the gigabytes are"),
    ("-r --ext=bin", "every executable in the tree"),
    ("-r --ext=dir", "directory skeleton"),
    ("--eext=small", "hide everything under 1M"),
    ("--eext=sh,py,exe", "everything but scripts and .exe"),
    ("-r --eext=o,d,rlib,pyc", "tree without build artefacts"),
];

#[derive(Debug, Clone, PartialEq)]
enum Atom {
    Dir,
    File,
    Bin,
    Hidden,
    Link,
    Broken,
    Ro,
    Locked,
    Suid,
    Empty,
    /// Size in [lo, hi); hi == u64::MAX is open-ended.
    Size(u64, u64),
    /// Lowercase extension without the dot; may be multi-part (tar.gz).
    Ext(String),
}

fn band(k: &str) -> Option<(u64, u64)> {
    let span = |a: usize, b: usize| Some((BANDS[a].lo, BANDS[b].hi));
    match k {
        "small" => span(0, 2),
        "mid" => span(3, 5),
        "large" => span(6, 8),
        _ => BANDS.iter().find(|b| b.key == k).map(|b| (b.lo, b.hi)),
    }
}

impl Atom {
    fn parse(raw: &str) -> Result<Atom, String> {
        let s = raw.trim().to_lowercase();
        if s.is_empty() {
            return Err("empty filter term".into());
        }
        if s.contains(['*', '?', '/', '\\', '[', ']']) {
            return Err(format!(
                "'{raw}': --ext/--eext take extensions, not patterns (e.g. mp3, tar.gz)"
            ));
        }
        if let Some(e) = s.strip_prefix('.') {
            return if e.is_empty() || e.starts_with('.') {
                Err(format!("invalid filter term '{raw}'"))
            } else {
                Ok(Atom::Ext(e.into()))
            };
        }
        Ok(match s.as_str() {
            "dir" => Atom::Dir,
            "file" => Atom::File,
            "bin" | "exe" => Atom::Bin,
            "hidden" => Atom::Hidden,
            "link" => Atom::Link,
            "broken" => Atom::Broken,
            "ro" => Atom::Ro,
            "locked" => Atom::Locked,
            "suid" => Atom::Suid,
            "empty" => Atom::Empty,
            k => band(k).map_or_else(|| Atom::Ext(k.into()), |(lo, hi)| Atom::Size(lo, hi)),
        })
    }

    fn matches(&self, n: &Node) -> bool {
        let file = !n.is_dir;
        match self {
            Atom::Dir => n.is_dir,
            Atom::File => n.kind == Kind::File,
            Atom::Bin => file && matches!(n.access, Access::Exec | Access::ExecWrite),
            Atom::Hidden => n.name.starts_with('.'),
            Atom::Link => n.kind == Kind::Symlink,
            Atom::Broken => n.broken,
            // Exec is only assigned when the file is not writable.
            Atom::Ro => matches!(
                n.access,
                Access::NoWrite | Access::DirReadOnly | Access::Exec
            ),
            Atom::Locked => n.access == Access::NoRead,
            Atom::Suid => n.setuid || n.setgid,
            Atom::Empty => file && n.size == Some(0),
            Atom::Size(lo, hi) => n
                .size
                .is_some_and(|s| s >= *lo && (s < *hi || *hi == u64::MAX)),
            // Case-insensitive `\.ext$` on files: mp3 matches a.mp3, A.MP3, .mp3.
            Atom::Ext(e) => {
                file && {
                    let name = n.name.to_lowercase();
                    name.len() > e.len()
                        && name.ends_with(e.as_str())
                        && name.as_bytes()[name.len() - e.len() - 1] == b'.'
                }
            }
        }
    }
}

/// All atoms must match (`a+b`).
type Term = Vec<Atom>;

fn hit(ts: &[Term], n: &Node) -> bool {
    ts.iter().any(|t| t.iter().all(|a| a.matches(n)))
}

pub struct Filter {
    inc: Vec<Term>,
    exc: Vec<Term>,
}

impl Filter {
    /// `None` when neither `--ext` nor `--eext` was given.
    pub fn parse(ext: &[String], eext: &[String]) -> Result<Option<Filter>, String> {
        if ext.is_empty() && eext.is_empty() {
            return Ok(None);
        }
        let terms = |v: &[String]| -> Result<Vec<Term>, String> {
            v.iter()
                .map(|t| t.split('+').map(Atom::parse).collect())
                .collect()
        };
        Ok(Some(Filter {
            inc: terms(ext)?,
            exc: terms(eext)?,
        }))
    }

    /// `--ext` asks for dotfiles, so the scan must include them.
    pub fn wants_hidden(&self) -> bool {
        self.inc.iter().flatten().any(|a| *a == Atom::Hidden)
    }

    /// Drop non-matching entries below `root`; the count lands in `root.filtered`.
    ///
    /// Tree view: a directory stays if it matches `--ext`, still holds entries
    /// after filtering, or only `--eext` was given. A directory matching `--eext`
    /// goes with its subtree.
    pub fn apply(&self, root: &mut Node, tree: bool) {
        root.filtered += self.prune(root, tree);
    }

    fn prune(&self, node: &mut Node, tree: bool) -> u64 {
        let Some(kids) = node.children.take() else {
            return 0;
        };
        let mut gone = 0;
        let mut keep = Vec::with_capacity(kids.len());
        for mut k in kids {
            let ok = if tree && k.children.is_some() {
                !hit(&self.exc, &k) && {
                    gone += self.prune(&mut k, tree);
                    self.inc.is_empty()
                        || hit(&self.inc, &k)
                        || k.children.as_ref().is_some_and(|c| !c.is_empty())
                }
            } else {
                (self.inc.is_empty() || hit(&self.inc, &k)) && !hit(&self.exc, &k)
            };
            if ok {
                keep.push(k);
            } else {
                gone += 1;
            }
        }
        node.children = Some(keep);
        gone
    }
}

/// `fyi --legend`: filter terms, examples and both colour scales.
pub fn legend(p: Painter) -> String {
    let h = |s: &str| p.paint(s, Sty::default().bold());
    let d = |s: &str| p.paint(s, Sty::dim());
    // Colour names are always printed, so the page works without colour too.
    let sw = |st: Sty| {
        if p.on {
            format!("{}  ", p.paint("■■", st))
        } else {
            String::new()
        }
    };
    let mut l: Vec<String> = vec![
        h("FILTERS"),
        "  --ext=LIST   list only entries matching any term".into(),
        "  --eext=LIST  hide entries matching any term (wins over --ext)".into(),
        d("  Comma-separated, case-insensitive; a+b must match both. Flags repeat."),
        d("  Flat view shows directories only with --ext=dir; tree view keeps"),
        d("  directories that still hold matches."),
        String::new(),
        h("Keywords"),
    ];
    l.extend(KEYWORDS.iter().map(|&(k, v)| format!("  {k:<9}{v}")));
    l.extend([
        String::new(),
        format!("{}  {}", h("Sizes"), d("(files; directories only with -d)")),
        "  small    < 1M      = s1 s2 s3".into(),
        "  mid      1M – 1G   = m1 m2 m3".into(),
        "  large    ≥ 1G      = l1 l2 l3".into(),
        "  s1 … l3  one band, see SIZE COLORS".into(),
        String::new(),
        h("Extensions"),
        "  mp3      Song.mp3, Song.MP3".into(),
        "  tar.gz   multi-part extension".into(),
        "  .bin     extension, even when it is a keyword".into(),
        d("  Every other word is an extension. No wildcards."),
        String::new(),
        h("Examples"),
    ]);
    l.extend(
        SUGGEST
            .iter()
            .map(|&(a, w)| format!("  fyi {a:<40}{}", d(w))),
    );
    l.extend([
        String::new(),
        format!(
            "{}  {}",
            h("SIZE COLORS"),
            d("(binary units; key = filter term)")
        ),
    ]);
    l.extend(BANDS.iter().map(|b| {
        format!(
            "  {}{:<3} {:<11} {}",
            sw(size_sty(b.lo)),
            b.key,
            b.range,
            b.color
        )
    }));
    l.extend([
        String::new(),
        format!(
            "{}  {}",
            h("NAME COLORS"),
            d("(your effective access; first match wins)")
        ),
    ]);
    let access = [
        (
            Sty::fg(AC_NO_READ),
            "wine red",
            "not readable / dir not enterable",
        ),
        (Sty::fg(AC_DIR_RO), "orange", "directory, not writable"),
        (
            Sty::fg(AC_DIR_FULL),
            "light yellow",
            "directory, full access",
        ),
        (
            Sty::fg(AC_EXEC_WRITE),
            "light green",
            "file, executable + writable",
        ),
        (Sty::fg(AC_EXEC), "green", "file, executable"),
        (Sty::fg(AC_NO_WRITE), "muted red", "file, not writable"),
        (
            Sty::fg(AC_NO_READ).strike(),
            "wine red, struck",
            "broken symlink",
        ),
        (Sty::default(), "default", "everything else"),
    ];
    l.extend(
        access
            .iter()
            .map(|&(st, c, w)| format!("  {}{c:<18}{w}", sw(st))),
    );
    l.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, size: u64) -> Node {
        let mut n = Node::new(name.into(), Kind::File);
        n.size = Some(size);
        n
    }

    fn dir(name: &str, kids: Vec<Node>) -> Node {
        let mut n = Node::new(name.into(), Kind::Dir);
        n.is_dir = true;
        n.access = Access::DirFull;
        n.children = Some(kids);
        n
    }

    fn names(n: &Node) -> Vec<&str> {
        n.children
            .iter()
            .flatten()
            .map(|c| c.name.as_str())
            .collect()
    }

    #[test]
    fn keywords_parse_as_keywords() {
        for (k, _) in KEYWORDS {
            assert!(!matches!(Atom::parse(k), Ok(Atom::Ext(_))), "{k}");
        }
        for k in ["small", "mid", "large", "s1", "m2", "l3", "exe"] {
            assert!(!matches!(Atom::parse(k), Ok(Atom::Ext(_))), "{k}");
        }
        assert_eq!(Atom::parse(".bin"), Ok(Atom::Ext("bin".into())));
        assert_eq!(Atom::parse("MP3"), Ok(Atom::Ext("mp3".into())));
        assert!(Atom::parse("").is_err());
        assert!(Atom::parse(".").is_err());
    }

    #[test]
    fn extensions() {
        let m = |t: &str, n: &str| Atom::parse(t).unwrap().matches(&node(n, 1));
        assert!(m("mp3", "Song.MP3"));
        assert!(m("mp3", "a.Mp3") && m("mp3", ".mp3"));
        assert!(!m("mp3", "mp3") && !m("mp3", "amp3"));
        assert!(!m("mp3", "a.mp34"));
        assert!(m("gz", "a.tar.gz") && m("tar.gz", "a.tar.gz"));
        assert!(!m("tar.gz", "atar.gz"));
        assert!(m(".bin", "fw.bin"));
        assert!(!Atom::parse("mp3").unwrap().matches(&dir("x.mp3", vec![])));
    }

    #[test]
    fn rejects_patterns() {
        for t in ["*.mp3", "mp?", "target/", "a\\b", "[ab]"] {
            assert!(Atom::parse(t).is_err(), "{t}");
        }
    }

    #[test]
    fn sizes_and_terms() {
        let f = Filter::parse(&["mkv+large".into()], &[]).unwrap().unwrap();
        assert!(hit(&f.inc, &node("a.mkv", 2 << 30)));
        assert!(!hit(&f.inc, &node("a.mkv", 1 << 20)));
        assert!(!hit(&f.inc, &node("a.mp4", 2 << 30)));
        assert!(Atom::parse("s1").unwrap().matches(&node("x", 0)));
        assert!(!Atom::parse("s1").unwrap().matches(&node("x", 1024)));
        assert!(Atom::parse("l3").unwrap().matches(&node("x", u64::MAX)));
        assert!(
            Atom::parse("small")
                .unwrap()
                .matches(&node("x", (1 << 20) - 1))
        );
        assert!(Atom::parse("mid").unwrap().matches(&node("x", 1 << 20)));
    }

    #[test]
    fn flat_include_exclude() {
        let mut root = dir(
            ".",
            vec![
                dir("music", vec![]),
                node("a.mp3", 1),
                node("b.sh", 1),
                node("c.py", 1),
            ],
        );
        let f = Filter::parse(&["dir".into(), "mp3".into()], &[])
            .unwrap()
            .unwrap();
        f.apply(&mut root, false);
        assert_eq!(names(&root), ["music", "a.mp3"]);
        assert_eq!(root.filtered, 2);

        let mut root = dir(".", vec![dir("d", vec![]), node("b.sh", 1), node("x", 1)]);
        let f = Filter::parse(&[], &["sh".into()]).unwrap().unwrap();
        f.apply(&mut root, false);
        assert_eq!(names(&root), ["d", "x"]);
    }

    #[test]
    fn tree_prunes_empty_dirs() {
        let tree = || {
            dir(
                ".",
                vec![
                    node("top.mp3", 1),
                    dir("has", vec![node("a.mp3", 1), node("b.txt", 1)]),
                    dir("none", vec![node("c.txt", 1)]),
                ],
            )
        };
        let mut root = tree();
        let f = Filter::parse(&["mp3".into()], &[]).unwrap().unwrap();
        f.apply(&mut root, true);
        assert_eq!(names(&root), ["top.mp3", "has"]);
        assert_eq!(names(&root.children.as_ref().unwrap()[1]), ["a.mp3"]);
        assert_eq!(root.filtered, 3); // b.txt, c.txt, none/

        let mut root = tree();
        let f = Filter::parse(&["dir".into(), "mp3".into()], &[])
            .unwrap()
            .unwrap();
        f.apply(&mut root, true);
        assert_eq!(names(&root), ["top.mp3", "has", "none"]);

        let mut root = tree();
        let f = Filter::parse(&[], &["txt".into()]).unwrap().unwrap();
        f.apply(&mut root, true);
        assert_eq!(names(&root), ["top.mp3", "has", "none"]);
    }

    #[test]
    fn legend_lists_every_band_and_keyword() {
        let s = legend(Painter { on: false });
        for b in &BANDS {
            assert!(s.contains(b.color) && s.contains(b.range), "{}", b.key);
        }
        for (k, _) in KEYWORDS {
            assert!(s.contains(k), "{k}");
        }
    }
}
