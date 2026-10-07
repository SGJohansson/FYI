mod history;
mod model;
mod render;
mod style;
mod text;
mod wsl;

use clap::{Parser, ValueEnum};
use history::Mode;
use model::{Order, ScanOpts, scan_root};
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
#[command(name = "fyi", version, about, max_term_width = 100)]
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
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && home.as_os_str().len() > 1
        && let Ok(rest) = norm.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", rest.display())
        };
    }
    s
}

fn emit(s: &str) -> bool {
    let mut o = std::io::stdout().lock();
    o.write_all(s.as_bytes()).and_then(|_| o.flush()).is_ok()
}

fn main() -> ExitCode {
    let cli = Cli::parse();
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

    if cli.hist {
        let list = history::list();
        if list.is_empty() {
            eprintln!("fyi: history is empty");
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
            Ok(e) => {
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
                eprintln!("fyi: {m}");
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
    let sopts = ScanOpts {
        all: cli.all,
        depth: if tree {
            cli.level.unwrap_or(usize::MAX).max(1)
        } else {
            1
        },
        limit: (tree && cli.limit > 0).then_some(cli.limit),
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
        let root = scan_root(&t.path, t.shown.clone(), &sopts);
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
        eprintln!("fyi: {e}");
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
