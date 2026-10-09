// fyi - for your information
// Copyright (C) 2026 S.G.Johansson <s.johansson.it@gmail.com>
// https://voidflow.tech/
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// A copy is included in the LICENSE file, or see
// http://www.apache.org/licenses/LICENSE-2.0
//
//! fyi: tell me about these files. Entry point, argument handling, WSL
//! target resolution and history commands.

#[cfg(not(unix))]
compile_error!("fyi is Linux/WSL only: build it inside WSL (cargo build in a Linux shell).");

/// Name this binary was invoked as (`lsi`), for usage and messages.
static PROG: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn prog() -> &'static str {
    PROG.get().map(String::as_str).unwrap_or("lsi")
}

/// `eprintln!` prefixed with the program name.
macro_rules! err {
    ($($t:tt)*) => {
        eprintln!("{}: {}", $crate::prog(), format_args!($($t)*))
    };
}

mod convert;
mod filter;
mod history;
mod model;
mod render;
mod style;
mod text;
mod wsl;

use clap::{CommandFactory, FromArgMatches, Parser, ValueEnum};
use convert::{Shell, To};
use filter::Filter;
use history::Mode;
use model::{Order, ScanOpts, apply_limit, scan_root};
use render::{Header, RenderOpts, render_flat, render_tree};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use style::Painter;
use wsl::{Resolve, Wsl};

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum ColorMode {
    Auto,
    Always,
    Never,
}

/// Tell me about these files: a colorful, WSL2-aware ls/tree replacement.
#[derive(Parser)]
#[command(
    name = "fyi",
    version,
    author = "S.G.Johansson <s.johansson.it@gmail.com>  https://voidflow.tech/",
    about,
    max_term_width = 100,
    help_template = "\
{name} {version}: {about}
by {author}

{usage-heading} {usage}

{all-args}{after-help}",
    after_help = "Filter keywords, size bands and color legend: lsi --legend\n\
cd to an Explorer path, unquoted or from the clipboard: eval \"$(lsi --init bash)\", then wcd\n\
Source and issues: https://github.com/SGJohansson/FYI  (Apache-2.0)"
)]
struct Cli {
    /// Paths to list. Windows paths (C:\..., C:/..., \\wsl.localhost\...) work on WSL,
    /// even unquoted.
    paths: Vec<String>,

    /// Show hidden entries (dotfiles).
    #[arg(short = 'a', long)]
    all: bool,

    /// One entry per line.
    #[arg(short = '1', long)]
    single: bool,

    /// Never shorten long names.
    #[arg(short = 'f', long = "full-names")]
    full_names: bool,

    /// Recursive tree view.
    #[arg(short = 'r', long = "tree", short_alias = 'R', alias = "recursive")]
    tree: bool,

    /// Max tree depth (implies --tree).
    #[arg(short = 'L', long = "level", value_name = "N")]
    level: Option<usize>,

    /// Max entries shown per directory in tree view (0 = no limit).
    #[arg(short = 'n', long = "limit", value_name = "N", default_value_t = 200)]
    limit: usize,

    /// Compute directory sizes (walks every subtree; can be slow, especially on /mnt/*).
    #[arg(short = 'd', long = "dir-sizes")]
    dir_sizes: bool,

    /// List only entries matching any term: keywords (dir, bin, hidden, small, mid,
    /// large, s1…l3, …) or file extensions (mp3, tar.gz). a+b = both. See --legend.
    #[arg(long = "ext", value_name = "LIST", value_delimiter = ',')]
    ext: Vec<String>,

    /// Hide entries matching any term (same terms as --ext; wins over --ext).
    #[arg(long = "eext", value_name = "LIST", value_delimiter = ',')]
    eext: Vec<String>,

    /// Show filter keywords, examples and what the size and name colors mean.
    #[arg(long = "legend")]
    legend: bool,

    /// Replay a previous listing: 1 = most recent.
    #[arg(short = 'b', long = "back", value_name = "N", num_args = 0..=1, default_missing_value = "1")]
    back: Option<usize>,

    /// List stored history.
    #[arg(long = "hist")]
    hist: bool,

    /// Do not record this listing in history.
    #[arg(long = "no-record")]
    no_record: bool,

    /// Max bytes stored per history entry (larger listings are pruned).
    #[arg(long = "hist-bytes", value_name = "BYTES", default_value_t = 1 << 20)]
    hist_bytes: usize,

    /// Convert a path between Windows and WSL and print it: D:\My Files ⇄ /mnt/d/My Files.
    /// Everything after -p is the path, even unquoted with spaces; put other options
    /// first. No path = current directory; - = read stdin, one path per line.
    #[arg(
        short = 'p',
        long = "wslpath",
        value_name = "PATH",
        num_args = 0..,
        allow_hyphen_values = true
    )]
    wslpath: Option<Vec<String>>,

    /// Convert the path(s) in the Windows clipboard (Explorer "Copy as path" or Ctrl+C on files).
    #[arg(long)]
    paste: bool,

    /// Also put the converted path in the Windows clipboard (with -p / --paste).
    #[arg(long)]
    copy: bool,

    /// Output format for -p / --paste.
    #[arg(long, value_enum, value_name = "FORMAT", default_value_t = To::Auto)]
    to: To,

    /// Shell-quote the converted path, ready to paste into a command line.
    #[arg(short = 'q', long)]
    quote: bool,

    /// Print the shell functions wcd (cd to any path) and wcp (copy as D:\...).
    #[arg(long, value_enum, value_name = "SHELL")]
    init: Option<Shell>,

    /// When to use color.
    #[arg(long, value_enum, default_value_t = ColorMode::Auto)]
    color: ColorMode,

    /// Override terminal width.
    #[arg(short = 'w', long, value_name = "COLS")]
    width: Option<usize>,
}

fn term_width() -> Option<usize> {
    // SAFETY: TIOCGWINSZ fills a zeroed winsize; failure leaves it untouched.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO, libc::STDIN_FILENO] {
        if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 {
            return Some(ws.ws_col as usize);
        }
    }
    std::env::var("COLUMNS").ok()?.parse().ok()
}

struct Target {
    path: PathBuf,
    shown: String,
    win: Option<String>,
}

/// Turn raw args into targets. On WSL, Windows-shaped args are converted; an
/// unquoted path with spaces arrives split, so failed drive args are retried
/// joined with the following args.
fn resolve_targets(args: &[String], wsl: Option<&Wsl>, errs: &mut Vec<String>) -> Vec<Target> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        let p = Path::new(arg);
        if p.symlink_metadata().is_ok() || wsl.is_none() || !wsl::looks_windows(arg) {
            let win = wsl.and_then(|w| absolute(p).and_then(|a| w.to_windows(&a)));
            out.push(Target {
                path: p.to_path_buf(),
                shown: display_path(arg),
                win,
            });
            continue;
        }
        let w = wsl.unwrap();
        let mut res = w.resolve(arg);
        if matches!(res, Resolve::NotFound(_)) && wsl::split_drive(arg).is_some() {
            for extra in 1..=4.min(args.len() - i) {
                let joined = args[i - 1..i + extra].join(" ");
                let r = w.resolve(&joined);
                if matches!(r, Resolve::Found { .. }) {
                    res = r;
                    i += extra;
                    break;
                }
            }
        }
        match res {
            Resolve::Found { path, win } => {
                let shown = display_path(&path.display().to_string());
                out.push(Target { path, shown, win });
            }
            Resolve::Ambiguous(c) => {
                let list: Vec<String> = c.iter().map(|p| format!("  {}", p.display())).collect();
                errs.push(format!(
                    "{arg}: ambiguous, quote the path. Candidates:\n{}",
                    list.join("\n")
                ));
            }
            Resolve::NotFound(m) => errs.push(m),
            Resolve::NotWindows => errs.push(format!("{arg}: not found")),
        }
    }
    out
}

fn absolute(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()
}

/// Absolute, `.`/`..`-normalised (lexically, symlinks kept), `$HOME` → `~`.
fn display_path(arg: &str) -> String {
    let Ok(abs) = std::path::absolute(arg) else {
        return arg.to_string();
    };
    let mut norm = PathBuf::new();
    for c in abs.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                norm.pop();
            }
            c => norm.push(c),
        }
    }
    let s = norm.display().to_string();
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.as_os_str().len() > 1);
    match home.as_deref().and_then(|h| norm.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => s,
    }
}

fn emit(s: &str) -> bool {
    let mut o = std::io::stdout().lock();
    o.write_all(s.as_bytes()).and_then(|_| o.flush()).is_ok()
}

/// `-p` / `--paste`: convert and print one line per path.
fn run_convert(cli: &Cli) -> ExitCode {
    let listing = !cli.paths.is_empty()
        || cli.tree
        || cli.level.is_some()
        || cli.back.is_some()
        || cli.hist
        || cli.legend
        || !cli.ext.is_empty()
        || !cli.eext.is_empty();
    if listing {
        err!(
            "-p / --paste cannot be combined with listing options; \
             put options before -p, everything after it is the path"
        );
        return ExitCode::from(2);
    }
    if cli.paste && cli.wslpath.is_some() {
        err!("use either -p or --paste");
        return ExitCode::from(2);
    }
    let Some(w) = Wsl::detect() else {
        err!("path conversion needs WSL");
        return ExitCode::from(2);
    };
    let args = cli.wslpath.clone().unwrap_or_default();
    if let Some(a) = args.first().filter(|a| a.len() > 1 && a.starts_with('-')) {
        err!(
            "{a}: everything after -p is the path; options go first: {} [OPTIONS] -p PATH",
            prog()
        );
        return ExitCode::from(2);
    }
    let text = if cli.paste {
        convert::paste(&w)
    } else if args.len() == 1 && args[0] == "-" {
        convert::read_stdin()
    } else if args.is_empty() {
        Ok(".".into())
    } else {
        // The shell split an unquoted path on its spaces: put it back together.
        Ok(args.join(" "))
    };
    let text = match text {
        Ok(t) => t,
        Err(m) => {
            err!("{m}");
            return ExitCode::FAILURE;
        }
    };
    let multi = cli.paste || args.first().is_some_and(|a| a == "-");
    let inputs: Vec<&str> = if multi {
        text.lines().filter(|l| !l.trim().is_empty()).collect()
    } else {
        vec![text.as_str()]
    };

    let mut ok = Vec::new();
    let mut failed = inputs.is_empty();
    for i in inputs {
        match convert::convert(&w, i, cli.to) {
            Ok(s) => ok.push(s),
            Err(m) => {
                err!("{m}");
                failed = true;
            }
        }
    }
    let mut out = String::new();
    for s in &ok {
        out.push_str(&if cli.quote {
            convert::shell_quote(s)
        } else {
            s.clone()
        });
        out.push('\n');
    }
    emit(&out);
    if cli.copy && !ok.is_empty() {
        if let Err(m) = convert::copy(&w, &ok.join("\r\n")) {
            err!("{m}");
            failed = true;
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn main() -> ExitCode {
    // `--wslpath=D:My Files` would only give clap the first word; split it so
    // everything after the flag is taken as the path, as with `-p`.
    let argv = std::env::args_os().flat_map(|a| {
        match a.to_str().and_then(|s| s.strip_prefix("--wslpath=")) {
            Some(v) => vec!["--wslpath".into(), v.into()],
            None => vec![a],
        }
    });
    let argv: Vec<std::ffi::OsString> = argv.collect();
    let name = argv
        .first()
        .and_then(|a| Path::new(a).file_name())
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("lsi")
        .to_string();
    let _ = PROG.set(name);
    let cli = Cli::command().bin_name(prog()).get_matches_from(argv);
    let cli = Cli::from_arg_matches(&cli).unwrap_or_else(|e| e.exit());
    let tty = std::io::stdout().is_terminal();
    let color = match cli.color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => tty && std::env::var_os("NO_COLOR").is_none(),
    };
    let width = cli
        .width
        .or_else(|| tty.then(term_width).flatten())
        .unwrap_or(80)
        .max(20);
    let ropts = RenderOpts {
        width,
        full_names: cli.full_names,
        single: cli.single,
        painter: Painter { on: color },
        plain: !tty && cli.color != ColorMode::Always && cli.width.is_none(),
    };
    let p = ropts.painter;
    let mut out = String::new();

    if let Some(sh) = cli.init {
        emit(&convert::init(sh, prog()));
        return ExitCode::SUCCESS;
    }
    let path_mode = cli.wslpath.is_some() || cli.paste;
    if path_mode {
        return run_convert(&cli);
    }
    if cli.copy || cli.quote || cli.to != To::Auto {
        err!("--copy, --quote and --to only apply to -p / --paste");
        return ExitCode::from(2);
    }

    if cli.legend {
        emit(&filter::legend(p));
        return ExitCode::SUCCESS;
    }
    let flt = match Filter::parse(&cli.ext, &cli.eext) {
        Ok(f) => f,
        Err(m) => {
            err!("{m}");
            return ExitCode::from(2);
        }
    };

    if cli.hist {
        let list = history::list();
        if list.is_empty() {
            err!("history is empty");
            return ExitCode::SUCCESS;
        }
        for (i, h) in list.iter().enumerate() {
            let mode = match h.mode {
                Mode::Flat => "list",
                Mode::Tree => "tree",
            };
            let win = h
                .win
                .as_deref()
                .map(|w| format!("  ⇄ {w}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "{:>3}  {}  {}  {}{}  {}\n",
                i + 1,
                p.paint(&history::fmt_time(h.ts), style::Sty::dim()),
                mode,
                p.paint(&h.path, style::Sty::default().bold()),
                p.paint(&win, style::Sty::dim()),
                p.paint(
                    &format!(
                        "{} entries{}",
                        h.entries,
                        if h.pruned { ", pruned" } else { "" }
                    ),
                    style::Sty::dim()
                ),
            ));
        }
        emit(&out);
        return ExitCode::SUCCESS;
    }

    if let Some(n) = cli.back {
        return match history::load(n.max(1)) {
            Ok(mut e) => {
                if let Some(f) = &flt {
                    f.apply(&mut e.root, matches!(e.head.mode, Mode::Tree));
                }
                let note = format!(
                    "⟲ #{n} · {}{}",
                    history::fmt_time(e.head.ts),
                    if e.head.pruned {
                        " · pruned to fit history limit"
                    } else {
                        ""
                    }
                );
                let h = Header {
                    path: &e.head.path,
                    win: e.head.win.as_deref(),
                    note: Some(note),
                };
                let ro = RenderOpts {
                    plain: false,
                    ..ropts
                };
                match e.head.mode {
                    Mode::Flat => render_flat(&e.root, &h, &ro, &mut out),
                    Mode::Tree => render_tree(&e.root, &h, &ro, &mut out),
                }
                emit(&out);
                ExitCode::SUCCESS
            }
            Err(m) => {
                err!("{m}");
                ExitCode::FAILURE
            }
        };
    }

    let wsl = Wsl::detect();
    let args = if cli.paths.is_empty() {
        vec![".".to_string()]
    } else {
        cli.paths.clone()
    };
    let mut errs = Vec::new();
    let targets = resolve_targets(&args, wsl.as_ref(), &mut errs);

    let tree = cli.tree || cli.level.is_some();
    let mode = if tree { Mode::Tree } else { Mode::Flat };
    let lim = (tree && cli.limit > 0).then_some(cli.limit);
    let sopts = ScanOpts {
        all: cli.all || flt.as_ref().is_some_and(Filter::wants_hidden),
        depth: if tree {
            cli.level.unwrap_or(usize::MAX).max(1)
        } else {
            1
        },
        // With a filter, the limit applies to what survives it.
        limit: lim.filter(|_| flt.is_none()),
        dir_sizes: cli.dir_sizes,
        order: if tree {
            Order::FilesFirst
        } else {
            Order::DirsFirst
        },
    };
    let record = tty && !cli.no_record;
    let mut failed = !errs.is_empty();

    for (i, t) in targets.iter().enumerate() {
        if i > 0 && !ropts.plain {
            out.push('\n');
        }
        let mut root = scan_root(&t.path, t.shown.clone(), &sopts);
        if let Some(f) = &flt {
            f.apply(&mut root, tree);
            if let Some(l) = lim {
                apply_limit(&mut root, l);
            }
        }
        if root.error.is_some() {
            failed = true;
        }
        let h = Header {
            path: &t.shown,
            win: t.win.as_deref(),
            note: None,
        };
        match mode {
            Mode::Flat => render_flat(&root, &h, &ropts, &mut out),
            Mode::Tree => render_tree(&root, &h, &ropts, &mut out),
        }
        if record && root.error.is_none() {
            let key = absolute(&t.path)
                .map(|a| a.display().to_string())
                .unwrap_or_else(|| t.shown.clone());
            let _ = history::record(root, mode, &key, t.win.as_deref(), cli.hist_bytes);
        }
    }
    emit(&out);
    for e in &errs {
        err!("{e}");
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
