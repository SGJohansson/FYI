//! ANSI truecolor styling and the fixed fyi palette.

use crate::model::Access;

pub type Rgb = (u8, u8, u8);

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Sty {
    pub fg: Option<Rgb>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub strike: bool,
}

impl Sty {
    pub const fn fg(rgb: Rgb) -> Self {
        Sty {
            fg: Some(rgb),
            bold: false,
            dim: false,
            italic: false,
            strike: false,
        }
    }
    pub const fn dim() -> Self {
        Sty {
            fg: None,
            bold: false,
            dim: true,
            italic: false,
            strike: false,
        }
    }
    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }
    pub const fn strike(mut self) -> Self {
        self.strike = true;
        self
    }
    fn is_plain(&self) -> bool {
        self.fg.is_none() && !self.bold && !self.dim && !self.italic && !self.strike
    }
}

#[derive(Clone, Copy)]
pub struct Painter {
    pub on: bool,
}

impl Painter {
    pub fn paint(&self, s: &str, st: Sty) -> String {
        if !self.on || st.is_plain() || s.is_empty() {
            return s.to_string();
        }
        let mut codes: Vec<String> = Vec::with_capacity(5);
        if st.bold {
            codes.push("1".into());
        }
        if st.dim {
            codes.push("2".into());
        }
        if st.italic {
            codes.push("3".into());
        }
        if st.strike {
            codes.push("9".into());
        }
        if let Some((r, g, b)) = st.fg {
            codes.push(format!("38;2;{r};{g};{b}"));
        }
        format!("\x1b[{}m{}\x1b[0m", codes.join(";"), s)
    }
}

// ---- size bands (binary units) -------------------------------------------

pub const SZ_LIGHT_BLUE: Rgb = (135, 206, 250); //       < 1K
pub const SZ_TEAL: Rgb = (0, 168, 150); //          1K – 100K
pub const SZ_LIGHT_TEAL: Rgb = (120, 230, 205); // 100K – 1M
pub const SZ_BLUE: Rgb = (70, 110, 255); //          1M – 10M
pub const SZ_PURPLE: Rgb = (165, 95, 235); //       10M – 100M
pub const SZ_PINK: Rgb = (255, 110, 190); //       100M – 1G
pub const SZ_RED: Rgb = (235, 60, 60); //            1G – 10G
pub const SZ_ORANGE: Rgb = (255, 150, 40); //       10G – 1T, bold ≥ 1T

const K: u64 = 1024;
const M: u64 = K * K;
const G: u64 = M * K;
const T: u64 = G * K;

pub fn size_sty(n: u64) -> Sty {
    match n {
        _ if n < K => Sty::fg(SZ_LIGHT_BLUE),
        _ if n < 100 * K => Sty::fg(SZ_TEAL),
        _ if n < M => Sty::fg(SZ_LIGHT_TEAL),
        _ if n < 10 * M => Sty::fg(SZ_BLUE),
        _ if n < 100 * M => Sty::fg(SZ_PURPLE),
        _ if n < G => Sty::fg(SZ_PINK),
        _ if n < 10 * G => Sty::fg(SZ_RED),
        _ if n < T => Sty::fg(SZ_ORANGE),
        _ => Sty::fg(SZ_ORANGE).bold(),
    }
}

// ---- access classes (colour of the name) ----------------------------------

pub const AC_NO_READ: Rgb = (170, 35, 70); // wine red
pub const AC_NO_WRITE: Rgb = (215, 135, 135); // muted red
pub const AC_EXEC: Rgb = (60, 190, 80); // green
pub const AC_EXEC_WRITE: Rgb = (150, 240, 150); // light green
pub const AC_DIR_RO: Rgb = (255, 165, 60); // orange
pub const AC_DIR_FULL: Rgb = (255, 240, 160); // light yellow

pub fn access_sty(a: Access, broken: bool) -> Sty {
    if broken {
        return Sty::fg(AC_NO_READ).strike();
    }
    match a {
        Access::Default => Sty::default(),
        Access::NoWrite => Sty::fg(AC_NO_WRITE),
        Access::Exec => Sty::fg(AC_EXEC),
        Access::ExecWrite => Sty::fg(AC_EXEC_WRITE),
        Access::NoRead => Sty::fg(AC_NO_READ),
        Access::DirReadOnly => Sty::fg(AC_DIR_RO),
        Access::DirFull => Sty::fg(AC_DIR_FULL),
    }
}

/// Tree guide colours, cycled per depth so nesting levels stay distinguishable.
pub const GUIDES: [Rgb; 4] = [
    (95, 95, 125),
    (80, 120, 108),
    (125, 95, 112),
    (115, 112, 82),
];

pub const ERR: Rgb = AC_NO_READ;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands() {
        assert_eq!(size_sty(0), Sty::fg(SZ_LIGHT_BLUE));
        assert_eq!(size_sty(1023), Sty::fg(SZ_LIGHT_BLUE));
        assert_eq!(size_sty(1024), Sty::fg(SZ_TEAL));
        assert_eq!(size_sty(100 * K), Sty::fg(SZ_LIGHT_TEAL));
        assert_eq!(size_sty(M), Sty::fg(SZ_BLUE));
        assert_eq!(size_sty(10 * M), Sty::fg(SZ_PURPLE));
        assert_eq!(size_sty(100 * M), Sty::fg(SZ_PINK));
        assert_eq!(size_sty(G), Sty::fg(SZ_RED));
        assert_eq!(size_sty(10 * G), Sty::fg(SZ_ORANGE));
        assert_eq!(size_sty(T), Sty::fg(SZ_ORANGE).bold());
    }

    #[test]
    fn painter_off_is_identity() {
        let p = Painter { on: false };
        assert_eq!(p.paint("x", Sty::fg(SZ_RED).bold()), "x");
    }
}
