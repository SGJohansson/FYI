// fyi - for your information
// Copyright (C) 2026 S.G.Johansson <s.johansson.it@gmail.com>
// https://voidflow.tech/
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// A copy is included in the LICENSE file, or see
// http://www.apache.org/licenses/LICENSE-2.0
//
//! Width-aware text helpers.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Human size, at most 5 columns: `612B`, `4.2K`, `38K`, `1.4G`.
pub fn human_size(n: u64) -> String {
    const UNITS: [char; 6] = ['K', 'M', 'G', 'T', 'P', 'E'];
    if n < 1024 {
        return format!("{n}B");
    }
    let mut v = n as f64 / 1024.0;
    let mut u = 0;
    while v >= 1023.95 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v < 9.95 {
        format!("{v:.1}{}", UNITS[u])
    } else {
        format!("{:.0}{}", v, UNITS[u])
    }
}

/// Shorten `s` to at most `max` columns with a middle ellipsis, keeping the
/// tail (and so the extension) visible: `a_very_long_n…me.tar.gz`.
pub fn truncate_middle(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    if max == 1 {
        return "…".into();
    }
    let budget = max - 1;
    let tail_w = (budget * 2 / 5).max(1);
    let head_w = budget - tail_w;

    let mut head = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw > head_w {
            break;
        }
        head.push(c);
        w += cw;
    }
    let mut tail: Vec<char> = Vec::new();
    let mut w = 0;
    for c in s.chars().rev() {
        let cw = c.width().unwrap_or(0);
        if w + cw > tail_w {
            break;
        }
        tail.push(c);
        w += cw;
    }
    tail.reverse();
    format!("{head}…{}", tail.into_iter().collect::<String>())
}

pub fn pad_left(s: &str, visible: usize, to: usize) -> String {
    format!("{}{}", " ".repeat(to.saturating_sub(visible)), s)
}

pub fn spaces(n: usize) -> String {
    " ".repeat(n)
}

pub fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(0), "0B");
        assert_eq!(human_size(1023), "1023B");
        assert_eq!(human_size(1024), "1.0K");
        assert_eq!(human_size(4300), "4.2K");
        assert_eq!(human_size(38 * 1024 + 100), "38K");
        assert_eq!(human_size(1023 * 1024 + 1000), "1.0M");
        assert_eq!(human_size(1_500_000_000), "1.4G");
        for n in [0u64, 999, 1024, 10_234, 1_048_575, u64::MAX] {
            assert!(human_size(n).len() <= 5, "{n} -> {}", human_size(n));
        }
    }

    #[test]
    fn truncation() {
        let s = "a_really_long_file_name_here.tar.gz";
        let t = truncate_middle(s, 20);
        assert_eq!(width(&t), 20);
        assert!(t.ends_with(".gz"));
        assert!(t.contains('…'));
        assert_eq!(truncate_middle("short", 20), "short");
        assert!(width(&truncate_middle("日本語のファイル名です.txt", 10)) <= 10);
    }
}
