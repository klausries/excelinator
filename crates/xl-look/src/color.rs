//! Colours as SpreadsheetML gives them (RGB, theme slot with tint, palette index,
//! automatic) and their resolution to plain RGB.

use crate::tree::{self, Node};

/// A colour as a file states it.
#[derive(Debug, Clone, PartialEq)]
pub enum Spec {
    /// `RRGGBB` (the alpha of ARGB dropped: Excel ignores it in cells), and a tint.
    Rgb(String, f64),
    /// Theme colour index (0 is `lt1`, 1 `dk1`, 2 `lt2`, 3 `dk2`, 4–9 the accents, 10
    /// `hlink`, 11 `folHlink`) and tint.
    Theme(u32, f64),
    /// Palette index and tint; 64 is the system foreground, 65 the system background.
    Indexed(u32, f64),
    /// `auto="1"`: the system colour of the context (black text, no fill).
    Auto,
}

impl Spec {
    /// The colour of a `<color>`, `<fgColor>`, `<bgColor>` element.
    pub fn of(n: &Node) -> Option<Spec> {
        let tint = n.get("tint").and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.0);
        if n.bool_attr("auto") == Some(true) {
            return Some(Spec::Auto);
        }
        if let Some(rgb) = n.get("rgb") {
            return hex6(rgb).map(|rgb| Spec::Rgb(rgb, tint));
        }
        if let Some(t) = n.get("theme").and_then(|v| v.parse().ok()) {
            return Some(Spec::Theme(t, tint));
        }
        if let Some(i) = n.get("indexed").and_then(|v| v.parse().ok()) {
            return Some(Spec::Indexed(i, tint));
        }
        None
    }
}

/// Excel's default palette (BIFF8, indexes 0–63), as openpyxl's `COLOR_INDEX` and
/// LibreOffice's import give it.
pub const DEFAULT_PALETTE: [&str; 64] = [
    "000000", "FFFFFF", "FF0000", "00FF00", "0000FF", "FFFF00", "FF00FF", "00FFFF", //
    "000000", "FFFFFF", "FF0000", "00FF00", "0000FF", "FFFF00", "FF00FF", "00FFFF", //
    "800000", "008000", "000080", "808000", "800080", "008080", "C0C0C0", "808080", //
    "9999FF", "993366", "FFFFCC", "CCFFFF", "660066", "FF8080", "0066CC", "CCCCFF", //
    "000080", "FF00FF", "FFFF00", "00FFFF", "800080", "800000", "008080", "0000FF", //
    "00CCFF", "CCFFFF", "CCFFCC", "FFFF99", "99CCFF", "FF99CC", "CC99FF", "FFCC99", //
    "3366FF", "33CCCC", "99CC00", "FFCC00", "FF9900", "FF6600", "666699", "969696", //
    "003366", "339966", "003300", "333300", "993300", "993366", "333399", "333333", //
];

/// The colour scheme of the Office theme (Office 2013 to 2022), in the order of the
/// theme's `clrScheme`: dk1, lt1, dk2, lt2, accent1–6, hlink, folHlink.
pub const OFFICE_SCHEME: [&str; 12] = ["000000", "FFFFFF", "44546A", "E7E6E6", "4472C4", "ED7D31", "A5A5A5", "FFC000", "5B9BD5", "70AD47", "0563C1", "954F72"];

/// What colours resolve against: the theme's scheme and the palette.
#[derive(Debug, Clone)]
pub struct Colors {
    /// In `clrScheme` order (dk1, lt1, dk2, lt2, …).
    pub scheme: [String; 12],
    pub palette: Vec<String>,
    /// The workbook has a theme part (else the Office theme is assumed).
    pub own_theme: bool,
}

impl Default for Colors {
    fn default() -> Self {
        Colors { scheme: OFFICE_SCHEME.map(str::to_string), palette: DEFAULT_PALETTE.iter().map(|s| s.to_string()).collect(), own_theme: false }
    }
}

/// `RRGGBB` in upper case from `RRGGBB` or `AARRGGBB`; `None` for anything else.
pub fn hex6(text: &str) -> Option<String> {
    let t = text.trim();
    if !matches!(t.len(), 6 | 8) || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(t[t.len() - 6..].to_ascii_uppercase())
}

/// The channel (0 red, 1 green, 2 blue) of `RRGGBB`; 0 where the text holds none.
pub fn channel(rgb: &str, i: usize) -> u8 {
    rgb.get(2 * i..2 * i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).unwrap_or(0)
}

impl Colors {
    /// From a theme part (`None`: the Office theme) and the file's own palette (empty:
    /// the default palette).
    pub fn new(theme: Option<&str>, palette: &[String]) -> Colors {
        let mut c = Colors::default();
        if let Some(scheme) = theme.and_then(|t| tree::parse(t).ok()).and_then(|t| t.find("clrScheme").cloned()) {
            let slots = ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"];
            for (i, slot) in slots.iter().enumerate() {
                let Some(n) = scheme.child(slot) else { continue };
                let rgb = n.child("srgbClr").and_then(|x| x.get("val")).or_else(|| n.child("sysClr").and_then(|x| x.get("lastClr")));
                if let Some(rgb) = rgb.and_then(hex6) {
                    c.scheme[i] = rgb;
                }
            }
            c.own_theme = true;
        }
        // An entry that is no colour leaves the default of its index (black beyond the
        // default palette, to keep the indexes of the entries after it).
        for (i, p) in palette.iter().enumerate() {
            match (hex6(p), i < c.palette.len()) {
                (Some(p), true) => c.palette[i] = p,
                (Some(p), false) => c.palette.push(p),
                (None, true) => {}
                (None, false) => c.palette.push("000000".into()),
            }
        }
        c
    }

    /// The RGB of a colour; `auto` is what automatic means in the context (`000000` for
    /// text and borders). `None`: automatic with nothing given for it, or a palette
    /// index beyond the palette.
    pub fn rgb(&self, spec: &Spec, auto: Option<&str>) -> Option<String> {
        let (base, tint) = match spec {
            Spec::Auto => return auto.map(str::to_string),
            Spec::Rgb(rgb, t) => (rgb.clone(), *t),
            Spec::Theme(i, t) => {
                // Theme indexes swap the light and dark pairs of the scheme.
                let slot = match *i {
                    0 => 1,
                    1 => 0,
                    2 => 3,
                    3 => 2,
                    n => n as usize,
                };
                (self.scheme.get(slot)?.clone(), *t)
            }
            Spec::Indexed(i, t) => match *i {
                64 => return auto.map(str::to_string).or(Some("000000".into())),
                65 => ("FFFFFF".into(), *t),
                n => (self.palette.get(n as usize)?.clone(), *t),
            },
        };
        Some(if tint == 0.0 { base } else { tinted(&base, tint) })
    }
}

/// The scale of hue, luminance and saturation in Excel's HLS conversion.
const HLSMAX: i64 = 240;

/// A colour lightened (tint > 0) or darkened (tint < 0) in HLS space, ECMA-376 Part 1
/// §18.3.1.15: lum' = lum·(1+tint) for a shade, lum·(1−tint) + HLSMAX·tint for a tint.
///
/// Excel does not publish its rounding. This is the integer conversion of the Windows
/// SDK (`ColorRGBToHLS`/`ColorHLSToRGB`, HLSMAX 240, every division truncating) with the
/// new luminance truncated: checked against what Excel itself displays for 93 colours
/// (theme, palette and RGB colours, tints from −0.5 to 0.8; `tools/excel_look.py`), all
/// equal. LibreOffice rounds differently and can be one or two steps off per channel.
pub fn tinted(rgb: &str, tint: f64) -> String {
    const M: i64 = HLSMAX;
    let (r, g, b) = (channel(rgb, 0) as i64, channel(rgb, 1) as i64, channel(rgb, 2) as i64);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let lum = ((max + min) * M + 255) / 510;
    let (hue, sat) = if max == min {
        (M * 2 / 3, 0)
    } else {
        let sat = if lum <= M / 2 { ((max - min) * M + (max + min) / 2) / (max + min) } else { ((max - min) * M + (510 - max - min) / 2) / (510 - max - min) };
        let delta = |c: i64| ((max - c) * (M / 6) + (max - min) / 2) / (max - min);
        let mut hue = if r == max {
            delta(b) - delta(g)
        } else if g == max {
            M / 3 + delta(r) - delta(b)
        } else {
            2 * M / 3 + delta(g) - delta(r)
        };
        if hue < 0 {
            hue += M;
        }
        if hue > M {
            hue -= M;
        }
        (hue, sat)
    };
    let l = if tint < 0.0 { lum as f64 * (1.0 + tint) } else { lum as f64 * (1.0 - tint) + M as f64 * tint };
    let l = (l as i64).clamp(0, M);
    if sat == 0 {
        let v = (l * 255 + M / 2) / M;
        return format!("{v:02X}{v:02X}{v:02X}");
    }
    let m2 = if l <= M / 2 { (l * (M + sat) + M / 2) / M } else { l + sat - (l * sat + M / 2) / M };
    let m1 = 2 * l - m2;
    // C's division truncates toward zero, as Rust's does for integers.
    let part = |h: i64| {
        let h = if h < 0 { h + M } else if h > M { h - M } else { h };
        if h < M / 6 {
            m1 + ((m2 - m1) * h + M / 12) / (M / 6)
        } else if h < M / 2 {
            m2
        } else if h < M * 2 / 3 {
            m1 + ((m2 - m1) * (M * 2 / 3 - h) + M / 12) / (M / 6)
        } else {
            m1
        }
    };
    let q = |h: i64| ((part(h) * 255 + M / 2) / M).clamp(0, 255);
    format!("{:02X}{:02X}{:02X}", q(hue + M / 3), q(hue), q(hue - M / 3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_indexes_swap_light_and_dark() {
        let c = Colors::default();
        assert_eq!(c.rgb(&Spec::Theme(0, 0.0), None).unwrap(), "FFFFFF");
        assert_eq!(c.rgb(&Spec::Theme(1, 0.0), None).unwrap(), "000000");
        assert_eq!(c.rgb(&Spec::Theme(3, 0.0), None).unwrap(), "44546A");
        assert_eq!(c.rgb(&Spec::Theme(4, 0.0), None).unwrap(), "4472C4");
    }

    #[test]
    fn tints_as_excel_shows_them() {
        // What Excel (Microsoft 365 for Mac) displays for theme colours of the Office
        // 2007 theme, palette and RGB colours with tints (tools/excel_look.py, 2026-11-15).
        let cases = [
            ("1F497D", -0.5, "0F243E"),
            ("1F497D", -0.25, "16365C"),
            ("1F497D", 0.4, "538DD5"),
            ("1F497D", 0.8, "C5D9F1"),
            ("4F81BD", 0.8, "DCE6F1"),
            ("EEECE1", -0.5, "948A54"),
            ("F79646", -0.25, "E26B0A"),
            ("800080", 0.4, "FF1AFF"),
            ("FFFFFF", -0.5, "808080"),
            ("888888", -0.3, "5F5F5F"),
            ("000000", 0.4, "666666"),
            ("FEDCBA", -0.3, "FC9936"),
            ("C0FFEE", -0.3, "39FFC9"),
            ("010203", -0.3, "000102"),
        ];
        for (rgb, tint, want) in cases {
            assert_eq!(tinted(rgb, tint), want, "{rgb} {tint}");
        }
    }
}
