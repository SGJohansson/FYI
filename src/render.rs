// fyi - for your information
// Copyright (C) 2026 S.G.Johansson <s.johansson.it@gmail.com>
// https://voidflow.tech/
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// A copy is included in the LICENSE file, or see
// http://www.apache.org/licenses/LICENSE-2.0
//
//! Rendering of a scanned `Node` as a flat grid, a single column, or a tree.

use crate::model::{Kind, Node};
use crate::style::{ERR, GUIDES, Painter, Sty, access_sty, size_sty};
use crate::text::{human_size, pad_left, plural, spaces, truncate_middle, width};

/// Width of the size field (`1023B`, `4.2K`, …).
const SZ: usize = 5;
/// Default cap for one name in grid mode.
const GRID_NAME_CAP: usize = 32;
/// Default cap for one name (incl. symlink target) in tree mode.
const TREE_NAME_CAP: usize = 56;
const GRID_SEP: usize = 2;

#[derive(Clone, Copy)]
pub struct RenderOpts {
    pub width: usize,
    pub full_names: bool,
    pub single: bool,
    pub painter: Painter,
    /// Not a terminal: bare names, one per line (flat mode only).
    pub plain: bool,
}

pub struct Header<'a> {
    pub path: &'a str,
    pub win: Option<&'a str>,
    /// Extra line above the listing (e.g. replay info).
    pub note: Option<String>,
}

struct Cell {
    text: String,
    w: usize,
}

fn name_cell(n: &Node, p: Painter, max: Option<usize>, show_target: bool) -> Cell {
    let base = access_sty(n.access, n.broken);
    let marks = format!(
        "{}{}",
        if n.setuid { "ˢ" } else { "" },
        if n.setgid { "ᵍ" } else { "" }
    );
    let slash = if n.is_dir && n.kind != Kind::Symlink {
        "/"
    } else {
        ""
    };

    let mut name = n.name.clone();
    let mut target = match (&n.link, n.kind) {
        (Some(t), Kind::Symlink) if show_target => {
            Some(format!("{t}{}", if n.is_dir { "/" } else { "" }))
        }
        _ => None,
    };
    let arrow = if n.kind == Kind::Symlink { " → " } else { "" };
    let arrow_shown = if n.kind == Kind::Symlink && target.is_none() {
        " →"
    } else {
        arrow
    };

    let total = |name: &str, target: &Option<String>| {
        width(name)
            + width(slash)
            + width(&marks)
            + width(arrow_shown)
            + target.as_deref().map(width).unwrap_or(0)
    };
    let overflow = max.filter(|&m| total(&name, &target) > m);
    if let Some(m) = overflow {
        if let Some(t) = &target {
            let tb = (m / 2).max(8);
            target = Some(truncate_middle(t, tb));
        }
        let over = total(&name, &target).saturating_sub(m);
        if over > 0 {
            let nb = width(&name).saturating_sub(over).max(4);
            name = truncate_middle(&name, nb);
        }
    }

    let w = total(&name, &target);
    let mut text = p.paint(&format!("{name}{slash}"), base);
    if !marks.is_empty() {
        text.push_str(&p.paint(&marks, Sty::dim()));
    }
    if n.kind == Kind::Symlink {
        text.push_str(&p.paint(arrow_shown, Sty::dim()));
        if let Some(t) = &target {
            let mut st = base;
            st.italic = true;
            text.push_str(&p.paint(t, st));
        }
    }
    Cell { text, w }
}

/// Size right-aligned in `SZ` columns; blank for directories without a size.
fn size_field(n: &Node, p: Painter, dash: bool) -> String {
    match n.size {
        Some(s) => {
            let h = human_size(s);
            pad_left(&p.paint(&h, size_sty(s)), width(&h), SZ)
        }
        None if dash => pad_left(&p.paint("-", Sty::dim()), 1, SZ),
        None => spaces(SZ),
    }
}

fn summary(n: &Node, p: Painter) -> String {
    if let Some(e) = &n.error {
        return p.paint(&format!("⚠ {e}"), Sty::fg(ERR));
    }
    if n.children.is_none() {
        return if n.is_dir && n.kind == Kind::Dir {
            p.paint("…", Sty::dim())
        } else {
            String::new()
        };
    }
    let (d, f) = n.counts();
    let mut s = format!(
        "{} · {}",
        plural(d, "dir", "dirs"),
        plural(f, "file", "files")
    );
    if n.filtered > 0 {
        s.push_str(&format!(" · {} filtered", n.filtered));
    }
    p.paint(&s, Sty::dim())
}

fn total_size(n: &Node) -> Option<u64> {
    n.size.or_else(|| {
        let kids = n.children.as_ref()?;
        Some(
            kids.iter()
                .filter(|c| !c.is_dir)
                .filter_map(|c| c.size)
                .sum(),
        )
    })
}

fn header_line(root: &Node, h: &Header, p: Painter, out: &mut String) {
    if let Some(note) = &h.note {
        out.push_str(&p.paint(note, Sty::dim()));
        out.push('\n');
    }
    let mut line = p.paint(h.path, Sty::default().bold());
    if let Some(w) = h.win {
        line.push_str(&p.paint(&format!("  ⇄ {w}"), Sty::dim()));
    }
    let s = summary(root, p);
    if !s.is_empty() {
        line.push_str("   ");
        line.push_str(&s);
    }
    let total = (root.error.is_none() && root.children.is_some())
        .then(|| total_size(root))
        .flatten();
    if let Some(t) = total {
        line.push_str(&p.paint(" · ", Sty::dim()));
        line.push_str(&p.paint(&human_size(t), size_sty(t)));
    }
    out.push_str(&line);
    out.push('\n');
}

fn more_line(n: &Node, p: Painter) -> String {
    let mut parts = Vec::new();
    if n.omitted_dirs > 0 {
        parts.push(plural(n.omitted_dirs, "dir", "dirs"));
    }
    if n.omitted_files > 0 {
        parts.push(plural(n.omitted_files, "file", "files"));
    }
    p.paint(
        &format!("… {} more ({})", n.omitted(), parts.join(", ")),
        Sty::dim(),
    )
}

// ---- flat ------------------------------------------------------------------

pub fn render_flat(root: &Node, h: &Header, o: &RenderOpts, out: &mut String) {
    let p = o.painter;
    if o.plain {
        match &root.children {
            Some(kids) => kids.iter().for_each(|c| {
                out.push_str(&c.name);
                out.push('\n');
            }),
            None => {
                out.push_str(h.path);
                out.push('\n');
            }
        }
        return;
    }

    let Some(kids) = &root.children else {
        if root.error.is_some() || root.is_dir {
            header_line(root, h, p, out);
        } else {
            // A single file argument: one line, full path as the name.
            if let Some(note) = &h.note {
                out.push_str(&p.paint(note, Sty::dim()));
                out.push('\n');
            }
            let mut n = root.clone();
            n.name = h.path.to_string();
            let max = (!o.full_names).then(|| o.width.saturating_sub(SZ + 2));
            let c = name_cell(&n, p, max, true);
            out.push_str(&format!("{}  {}", size_field(&n, p, true), c.text));
            if let Some(w) = h.win {
                out.push_str(&p.paint(&format!("  ⇄ {w}"), Sty::dim()));
            }
            out.push('\n');
        }
        return;
    };

    header_line(root, h, p, out);
    if kids.is_empty() && root.omitted() == 0 {
        return;
    }

    if o.single {
        let max = (!o.full_names).then(|| o.width.saturating_sub(SZ + 2));
        for c in kids {
            let cell = name_cell(c, p, max, true);
            out.push_str(&format!("{}  {}\n", size_field(c, p, true), cell.text));
        }
    } else {
        let max = (!o.full_names).then_some(GRID_NAME_CAP.min(o.width.saturating_sub(SZ + 1)));
        let cells: Vec<Cell> = kids
            .iter()
            .map(|c| {
                let nc = name_cell(c, p, max, false);
                Cell {
                    text: format!("{} {}", size_field(c, p, true), nc.text),
                    w: SZ + 1 + nc.w,
                }
            })
            .collect();
        grid(&cells, o.width, out);
    }
    if root.omitted() > 0 {
        out.push_str(&more_line(root, p));
        out.push('\n');
    }
}

/// Column-major layout with the fewest rows that fit `width`.
pub fn layout(widths: &[usize], width: usize, sep: usize) -> (usize, Vec<usize>) {
    let n = widths.len();
    if n == 0 {
        return (0, vec![]);
    }
    let min_w = widths.iter().copied().min().unwrap_or(1).max(1);
    let max_cols = (width + sep) / (min_w + sep);
    let start = n.div_ceil(max_cols.max(1)).max(1);
    for rows in start..=n {
        let cols = n.div_ceil(rows);
        let col_w: Vec<usize> = (0..cols)
            .map(|c| {
                widths[c * rows..((c + 1) * rows).min(n)]
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let total: usize = col_w.iter().sum::<usize>() + sep * (cols - 1);
        if total <= width || cols == 1 {
            return (rows, col_w);
        }
    }
    (n, vec![widths.iter().copied().max().unwrap_or(0)])
}

fn grid(cells: &[Cell], width: usize, out: &mut String) {
    let widths: Vec<usize> = cells.iter().map(|c| c.w).collect();
    let (rows, col_w) = layout(&widths, width, GRID_SEP);
    let n = cells.len();
    for r in 0..rows {
        let mut line = String::new();
        for (c, cw) in col_w.iter().enumerate() {
            let i = c * rows + r;
            if i >= n {
                break;
            }
            line.push_str(&cells[i].text);
            if i + rows < n {
                line.push_str(&spaces(cw - cells[i].w + GRID_SEP));
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
}

// ---- tree ------------------------------------------------------------------

enum TKind<'a> {
    Entry(&'a Node),
    More(&'a Node),
}

struct TLine<'a> {
    prefix: String,
    pw: usize,
    kind: TKind<'a>,
}

fn collect<'a>(node: &'a Node, lasts: &mut Vec<bool>, p: Painter, out: &mut Vec<TLine<'a>>) {
    let Some(kids) = &node.children else { return };
    let has_more = node.omitted() > 0;
    let total = kids.len() + usize::from(has_more);
    let depth = lasts.len();

    let mut base = String::new();
    for (i, last) in lasts.iter().enumerate() {
        base.push_str(&if *last {
            spaces(4)
        } else {
            p.paint("│   ", Sty::fg(GUIDES[i % GUIDES.len()]))
        });
    }
    let guide = Sty::fg(GUIDES[depth % GUIDES.len()]);

    for i in 0..total {
        let last = i + 1 == total;
        let prefix = format!(
            "{base}{}",
            p.paint(if last { "└── " } else { "├── " }, guide)
        );
        let pw = 4 * (depth + 1);
        if i < kids.len() {
            let k = &kids[i];
            out.push(TLine {
                prefix,
                pw,
                kind: TKind::Entry(k),
            });
            lasts.push(last);
            collect(k, lasts, p, out);
            lasts.pop();
        } else {
            out.push(TLine {
                prefix,
                pw,
                kind: TKind::More(node),
            });
        }
    }
}

pub fn render_tree(root: &Node, h: &Header, o: &RenderOpts, out: &mut String) {
    let p = o.painter;
    if let Some(note) = &h.note {
        out.push_str(&p.paint(note, Sty::dim()));
        out.push('\n');
    }
    let mut lines = Vec::new();
    collect(root, &mut Vec::new(), p, &mut lines);

    // Column where the size field starts.
    let cap = (!o.full_names).then(|| o.width.saturating_sub(SZ + 1 + 22).clamp(24, TREE_NAME_CAP));
    let cells: Vec<Option<Cell>> = lines
        .iter()
        .map(|l| match l.kind {
            TKind::Entry(n) => Some(name_cell(
                n,
                p,
                cap.map(|c| c.saturating_sub(l.pw).max(8)),
                true,
            )),
            TKind::More(_) => None,
        })
        .collect();
    let mut col = lines
        .iter()
        .zip(&cells)
        .filter_map(|(l, c)| c.as_ref().map(|c| l.pw + c.w))
        .max()
        .unwrap_or(0);
    let mut root_txt = p.paint(h.path, Sty::default().bold());
    let mut root_w = width(h.path);
    if let Some(w) = h.win {
        let s = format!("  ⇄ {w}");
        root_w += width(&s);
        root_txt.push_str(&p.paint(&s, Sty::dim()));
    }
    if let Some(c) = cap {
        col = col.min(c);
    }

    // Root line.
    out.push_str(&root_txt);
    if root.is_dir || root.error.is_some() {
        if root_w <= col {
            out.push_str(&spaces(col - root_w));
            out.push(' ');
            out.push_str(&size_field(root, p, false));
        } else {
            out.push(' ');
        }
        let s = summary(root, p);
        if !s.is_empty() {
            out.push_str("  ");
            out.push_str(&s);
        }
    } else {
        out.push_str("  ");
        out.push_str(&size_field(root, p, true));
    }
    out.push('\n');

    for (l, c) in lines.iter().zip(&cells) {
        out.push_str(&l.prefix);
        match (&l.kind, c) {
            (TKind::Entry(n), Some(c)) => {
                out.push_str(&c.text);
                out.push_str(&spaces(col.saturating_sub(l.pw + c.w)));
                out.push(' ');
                out.push_str(&size_field(n, p, false));
                if n.is_dir && n.kind == Kind::Dir {
                    let s = summary(n, p);
                    if !s.is_empty() {
                        out.push_str("  ");
                        out.push_str(&s);
                    }
                }
            }
            (TKind::More(n), _) => out.push_str(&more_line(n, p)),
            _ => {}
        }
        // Trim trailing blanks from lines that end on an empty field.
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_fits() {
        let w = vec![10; 9];
        let (rows, cols) = layout(&w, 80, 2);
        assert_eq!(rows, 2); // 6 columns of 10 + 5*2 = 70 ≤ 80; 9 entries → 2 rows (5 cols)
        assert!(cols.iter().sum::<usize>() + 2 * (cols.len() - 1) <= 80);
        let (rows, _) = layout(&[100, 5], 80, 2);
        assert_eq!(rows, 2);
        assert_eq!(layout(&[], 80, 2).0, 0);
    }
}
