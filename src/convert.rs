// fyi - for your information
// Copyright (C) 2026 S.G.Johansson <s.johansson.it@gmail.com>
// https://voidflow.tech/
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// A copy is included in the LICENSE file, or see
// http://www.apache.org/licenses/LICENSE-2.0
//
//! `fyi -p` / `--paste` / `--copy` / `--init`: convert paths between Windows
//! and WSL for `cd`, pipelines and the clipboard.
//!
//! Unlike `wslpath`, the input may arrive mangled by the shell (`C:\My Files`
//! typed unquoted reaches us as `C:My` + `Files`): all arguments are joined
//! and the separators are recovered against what exists on disk.

use crate::wsl::{self, Resolve, Wsl};
use clap::ValueEnum;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum To {
    /// Windows input → Linux path, Linux input → Windows path.
    Auto,
    /// /mnt/d/My Files
    Linux,
    /// D:\My Files
    Win,
    /// D:/My Files
    Mixed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
}

/// Strip what copying adds: CR/LF and the double quotes of Explorer's
/// "Copy as path" (or single quotes).
pub fn clean(s: &str) -> &str {
    let s = s.trim();
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return s[1..s.len() - 1].trim();
        }
    }
    s
}

pub fn convert(w: &Wsl, input: &str, to: To) -> Result<String, String> {
    let s = clean(input);
    if s.is_empty() {
        return Err("empty path".into());
    }
    if wsl::looks_windows(s) {
        from_windows(w, s, to)
    } else {
        from_linux(w, s, to)
    }
}

fn segs(rest: &str) -> Vec<&str> {
    rest.split(['\\', '/']).filter(|s| !s.is_empty()).collect()
}

fn from_windows(w: &Wsl, s: &str, to: To) -> Result<String, String> {
    let (linux, win) = match w.resolve(s) {
        Resolve::Found { path, win } => {
            let win = win.unwrap_or_else(|| s.replace('/', "\\"));
            (path, win)
        }
        Resolve::Ambiguous(c) => {
            let list: Vec<String> = c.iter().map(|p| format!("  {}", p.display())).collect();
            return Err(format!(
                "{s}: ambiguous, quote the path or use --paste. Candidates:\n{}",
                list.join("\n")
            ));
        }
        Resolve::NotFound(m) => match wsl::split_drive(s) {
            // Separators survived (quoted / pasted): convert as written, like
            // wslpath, so targets that do not exist yet still work.
            Some((l, rest)) if rest.contains(['\\', '/']) => {
                let sg = segs(rest);
                let linux = sg.iter().fold(w.mount_for(l), |p, x| p.join(x));
                let win = format!("{}:\\{}", l.to_ascii_uppercase(), sg.join("\\"));
                (linux, win)
            }
            Some(_) => {
                return Err(format!(
                    "{s}: not found. The shell removed the backslashes, so only existing \
                     paths can be recovered; quote the path or use --paste"
                ));
            }
            None => return Err(m),
        },
        Resolve::NotWindows => return Err(format!("{s}: not a Windows path")),
    };
    Ok(match to {
        To::Auto | To::Linux => linux.display().to_string(),
        To::Win => win,
        To::Mixed => win.replace('\\', "/"),
    })
}

/// Lexical absolute path: `.`/`..` resolved, symlinks kept.
fn lexical_abs(p: &Path) -> Option<PathBuf> {
    let abs = std::path::absolute(p).ok()?;
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    Some(out)
}

fn from_linux(w: &Wsl, s: &str, to: To) -> Result<String, String> {
    let p = Path::new(s);
    // A Linux path that does not exist but carries `\` or `:` is almost
    // certainly a Windows path glued to something else; do not invent a result.
    if p.symlink_metadata().is_err() && s.contains(['\\', ':']) {
        return Err(format!(
            "{s}: no such path, and it mixes Linux and Windows syntax"
        ));
    }
    let abs = std::fs::canonicalize(p)
        .ok()
        .or_else(|| lexical_abs(p))
        .ok_or_else(|| format!("{s}: cannot resolve"))?;
    let linux = abs.display().to_string();
    if to == To::Linux {
        return Ok(linux);
    }
    let win = match w.to_windows(&abs) {
        Some(x) => x,
        None => {
            let d = w.distro.as_deref().ok_or_else(|| {
                format!("{s}: not on a Windows drive and the distro name is unknown")
            })?;
            format!("\\\\wsl.localhost\\{d}{}", linux.replace('/', "\\"))
        }
    };
    Ok(if to == To::Mixed {
        win.replace('\\', "/")
    } else {
        win
    })
}

/// POSIX single-quoting, only when needed.
pub fn shell_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "/._-+:,@%=".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub fn read_stdin() -> Result<String, String> {
    let mut s = String::new();
    std::io::stdin()
        .read_to_string(&mut s)
        .map_err(|e| format!("stdin: {e}"))?;
    Ok(s)
}

/// A Windows executable: from PATH (interop), else its standard location.
fn win_exe(w: &Wsl, name: &str, dir: &str) -> PathBuf {
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            let p = d.join(name);
            if p.is_file() {
                return p;
            }
        }
    }
    w.mount_for('C').join(dir).join(name)
}

/// Clipboard text; falls back to files copied with Ctrl+C in Explorer.
pub fn paste(w: &Wsl) -> Result<String, String> {
    const PS: &str = "[Console]::OutputEncoding=[Text.Encoding]::UTF8; \
        $t = Get-Clipboard -Raw; \
        if (-not $t) { $t = (Get-Clipboard -Format FileDropList | ForEach-Object FullName) -join [char]10 }; \
        [Console]::Out.Write($t)";
    let ps = win_exe(
        w,
        "powershell.exe",
        "Windows/System32/WindowsPowerShell/v1.0",
    );
    let out = Command::new(&ps)
        .args(["-NoProfile", "-NonInteractive", "-Command", PS])
        // A Linux cwd shows up as a UNC path on the Windows side; avoid it.
        .current_dir(w.mount_for('C'))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("clipboard: cannot run {} ({e})", ps.display()))?;
    if !out.status.success() {
        return Err("clipboard: powershell.exe failed".into());
    }
    let s = String::from_utf8_lossy(&out.stdout).into_owned();
    if s.trim().is_empty() {
        return Err("clipboard is empty".into());
    }
    Ok(s)
}

/// Put `text` in the Windows clipboard via clip.exe (UTF-16LE with BOM, so
/// non-ASCII names survive).
pub fn copy(w: &Wsl, text: &str) -> Result<(), String> {
    let clip = win_exe(w, "clip.exe", "Windows/System32");
    let mut child = Command::new(&clip)
        .current_dir(w.mount_for('C'))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("clipboard: cannot run {} ({e})", clip.display()))?;
    let mut buf = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        buf.extend(u.to_le_bytes());
    }
    let wrote = child
        .stdin
        .take()
        .map(|mut i| i.write_all(&buf))
        .unwrap_or(Ok(()));
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    if wrote.is_err() || !ok {
        return Err("clipboard: clip.exe failed".into());
    }
    Ok(())
}

/// Shell functions printed by `fyi --init`. A child process cannot change the
/// shell's directory, so `wcd` has to be a function.
pub fn init(sh: Shell, prog: &str) -> String {
    match sh {
        // The same POSIX-style function works in bash and zsh.
        Shell::Bash | Shell::Zsh => {
            r#"# fyi shell integration: eval "$(@PROG@ --init bash)"  (or zsh)
# wcd [PATH...]  cd to a Windows or Linux path, typed unquoted or pasted;
#                no argument: the Windows clipboard. A file goes to its folder.
# wcp [PATH...]  copy PATH (default: here) to the Windows clipboard as D:\...
__fyi_usage() {
    case "$2" in
        -h|--help)
            printf '%s\n' \
                "usage: wcd [PATH]   cd to a Windows or Linux path; no PATH = Windows clipboard" \
                "       wcp [PATH]   copy PATH (default: here) to the Windows clipboard as D:\\..." \
                "PATH may be typed unquoted: wcd D:\\My Files\\Music" \
                "A drive root is C: (a trailing \\ makes bash wait for another line)."
            return 0 ;;
        -?*)
            printf '%s: %s: no options; see %s -h\n' "$1" "$2" "$1" >&2
            return 2 ;;
    esac
    return 1
}
wcd() {
    __fyi_usage wcd "$1"; case $? in 0) return 0 ;; 2) return 2 ;; esac
    local __fyi_p
    if [ "$#" -eq 0 ]; then
        __fyi_p=$(command @PROG@ --paste --to linux) || return
    else
        __fyi_p=$(command @PROG@ --to linux -p "$@") || return
    fi
    __fyi_p=${__fyi_p%%
*}
    if [ -f "$__fyi_p" ]; then __fyi_p=${__fyi_p%/*}; fi
    builtin cd -- "${__fyi_p:-/}"
}
wcp() {
    __fyi_usage wcp "$1"; case $? in 0) return 0 ;; 2) return 2 ;; esac
    command @PROG@ --copy --to win -p "$@"
}
"#
        }
    }
    .replace("@PROG@", prog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup(name: &str) -> (PathBuf, Wsl) {
        let root = std::env::temp_dir().join(format!("fyi-conv-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("My Files/Graphics/Designs")).unwrap();
        fs::create_dir_all(root.join("Kalles filer/lol")).unwrap();
        fs::write(root.join("Kalles filer/lol/vad e detta.mp3"), b"x").unwrap();
        let w = Wsl {
            drives: vec![('D', root.clone())],
            distro: Some("Ubuntu".into()),
        };
        (root, w)
    }

    #[test]
    fn explorer_paths() {
        let (root, w) = setup("explorer");
        let want = root.join("My Files/Graphics/Designs").display().to_string();
        // Pasted / quoted.
        assert_eq!(
            convert(&w, r"D:\My Files\Graphics\Designs", To::Auto).unwrap(),
            want
        );
        // Explorer "Copy as path".
        assert_eq!(
            convert(&w, "\"D:\\My Files\\Graphics\\Designs\"\r\n", To::Auto).unwrap(),
            want
        );
        // Typed unquoted: bash delivers D:My + FilesGraphicsDesigns, joined by main.
        assert_eq!(
            convert(&w, "D:My FilesGraphicsDesigns", To::Auto).unwrap(),
            want
        );
        // Case-insensitive, canonical Windows spelling back.
        assert_eq!(
            convert(&w, "d:my filesgraphicsdesigns", To::Win).unwrap(),
            r"D:\My Files\Graphics\Designs"
        );
        assert_eq!(
            convert(&w, "D:Kalles filerlolvad e detta.mp3", To::Auto).unwrap(),
            root.join("Kalles filer/lol/vad e detta.mp3")
                .display()
                .to_string()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_targets() {
        let (root, w) = setup("missing");
        // Quoted, not existing yet: lexical, like wslpath.
        assert_eq!(
            convert(&w, r"d:\New Dir\x", To::Auto).unwrap(),
            root.join("New Dir/x").display().to_string()
        );
        assert_eq!(
            convert(&w, r"d:\New Dir\x", To::Win).unwrap(),
            r"D:\New Dir\x"
        );
        // Mangled and not existing: cannot know where the separators were.
        assert!(convert(&w, "D:NewDirx", To::Auto).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn linux_to_windows() {
        let (root, w) = setup("l2w");
        let p = root.join("My Files/Graphics");
        assert_eq!(
            convert(&w, &p.display().to_string(), To::Auto).unwrap(),
            r"D:\My Files\Graphics"
        );
        assert_eq!(
            convert(&w, &p.display().to_string(), To::Mixed).unwrap(),
            "D:/My Files/Graphics"
        );
        let w2 = Wsl {
            drives: vec![],
            distro: Some("Ubuntu".into()),
        };
        assert_eq!(
            convert(&w2, "/etc/../usr/nonexistent", To::Auto).unwrap(),
            r"\\wsl.localhost\Ubuntu\usr\nonexistent"
        );
        assert_eq!(
            convert(&w2, r"\\wsl.localhost\Ubuntu\home\me", To::Auto).unwrap(),
            "/home/me"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn quoting_and_cleaning() {
        assert_eq!(shell_quote("/mnt/d/a_b-1.mp3"), "/mnt/d/a_b-1.mp3");
        assert_eq!(shell_quote("/mnt/d/My Files"), "'/mnt/d/My Files'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(r"D:\x"), r"'D:\x'");
        assert_eq!(clean("  \"C:\\x y\"\r\n"), r"C:\x y");
        assert_eq!(clean("'/a b'"), "/a b");
    }
}
