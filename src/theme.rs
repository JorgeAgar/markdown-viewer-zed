use gpui::{Hsla, WindowAppearance, rgb};

use crate::{highlight::Tok, markdown::Callout};

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    pub bg: Hsla,
    pub fg: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub link: Hsla,
    pub code_bg: Hsla,
    pub bar_bg: Hsla,
    pub hover: Hsla,
    pub accent: Hsla,
    pub note: Hsla,
    pub tip: Hsla,
    pub important: Hsla,
    pub warning: Hsla,
    pub caution: Hsla,
    pub syntax: Syntax,
}

/// Code colors, from Zed's One Dark / One Light syntax themes.
#[derive(Clone, Copy)]
pub struct Syntax {
    pub keyword: Hsla,
    pub ty: Hsla,
    pub function: Hsla,
    pub string: Hsla,
    pub number: Hsla,
    pub comment: Hsla,
    pub constant: Hsla,
    pub attr: Hsla,
    pub variable: Hsla,
    pub tag: Hsla,
    pub property: Hsla,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Theme {
    pub fn for_appearance(a: WindowAppearance) -> Self {
        match a {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
            _ => Self::dark(),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            bg: c(0x282c34),
            fg: c(0xdce0e5),
            muted: c(0x878a98),
            border: c(0x3d424d),
            link: c(0x74ade8),
            code_bg: c(0x2f343e),
            bar_bg: c(0x21242b),
            hover: c(0x363c46),
            accent: c(0x74ade8),
            note: c(0x74ade8),
            tip: c(0xa1c181),
            important: c(0xb477cf),
            warning: c(0xdec184),
            caution: c(0xd07277),
            syntax: Syntax {
                keyword: c(0xb477cf),
                ty: c(0x6eb4bf),
                function: c(0x73ade9),
                string: c(0xa1c181),
                number: c(0xbf956a),
                comment: c(0x878e98),
                constant: c(0xdfc184),
                attr: c(0x74ade8),
                variable: c(0xd07277),
                tag: c(0x74ade8),
                property: c(0xd07277),
            },
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            bg: c(0xfafafa),
            fg: c(0x242529),
            muted: c(0x7e8087),
            border: c(0xdcdcdd),
            link: c(0x5c78e2),
            code_bg: c(0xefeff0),
            bar_bg: c(0xebebec),
            hover: c(0xdfdfe0),
            accent: c(0x5c78e2),
            note: c(0x5c78e2),
            tip: c(0x669f59),
            important: c(0x9f4fc4),
            warning: c(0xa48819),
            caution: c(0xd3604f),
            syntax: Syntax {
                keyword: c(0xa449ab),
                ty: c(0x3882b7),
                function: c(0x5b79e3),
                string: c(0x649f57),
                number: c(0xad6e25),
                comment: c(0x8c8e94),
                constant: c(0xc18401),
                attr: c(0x5c78e2),
                variable: c(0xd3604f),
                tag: c(0x5c78e2),
                property: c(0xd3604f),
            },
        }
    }

    pub fn callout(&self, k: Callout) -> Hsla {
        match k {
            Callout::Note => self.note,
            Callout::Tip => self.tip,
            Callout::Important => self.important,
            Callout::Warning => self.warning,
            Callout::Caution => self.caution,
        }
    }

    pub fn token(&self, tok: Tok) -> Hsla {
        let s = &self.syntax;
        match tok {
            Tok::Keyword => s.keyword,
            Tok::Type => s.ty,
            Tok::Function => s.function,
            Tok::String => s.string,
            Tok::Number => s.number,
            Tok::Comment => s.comment,
            Tok::Constant => s.constant,
            Tok::Attr => s.attr,
            Tok::Variable => s.variable,
            Tok::Tag => s.tag,
            Tok::Property => s.property,
            Tok::Inserted => self.tip,
            Tok::Deleted => self.caution,
            Tok::Meta => self.muted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Rgba;

    const ALL_TOKS: [Tok; 14] = [
        Tok::Keyword,
        Tok::Type,
        Tok::Function,
        Tok::String,
        Tok::Number,
        Tok::Comment,
        Tok::Constant,
        Tok::Attr,
        Tok::Variable,
        Tok::Tag,
        Tok::Property,
        Tok::Inserted,
        Tok::Deleted,
        Tok::Meta,
    ];
    const ALL_CALLOUTS: [Callout; 5] = [
        Callout::Note,
        Callout::Tip,
        Callout::Important,
        Callout::Warning,
        Callout::Caution,
    ];

    /// Doesn't compile when a variant is added: add it to the lists above.
    #[allow(dead_code)]
    fn exhaustive(t: Tok, c: Callout) {
        match t {
            Tok::Keyword
            | Tok::Type
            | Tok::Function
            | Tok::String
            | Tok::Number
            | Tok::Comment
            | Tok::Constant
            | Tok::Attr
            | Tok::Variable
            | Tok::Tag
            | Tok::Property
            | Tok::Inserted
            | Tok::Deleted
            | Tok::Meta => {}
        }
        match c {
            Callout::Note
            | Callout::Tip
            | Callout::Important
            | Callout::Warning
            | Callout::Caution => {}
        }
    }

    fn themes() -> [(&'static str, Theme); 2] {
        [("light", Theme::light()), ("dark", Theme::dark())]
    }

    fn hex(c: Hsla) -> u32 {
        let c = Rgba::from(c);
        let b = |v: f32| (v * 255.).round() as u32;
        (b(c.r) << 16) | (b(c.g) << 8) | b(c.b)
    }

    /// WCAG 2 relative luminance.
    fn luminance(c: Hsla) -> f64 {
        let c = Rgba::from(c);
        assert!((c.a - 1.).abs() < 1e-6, "palette colors are opaque");
        let lin = |v: f32| {
            let v = v as f64;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    /// WCAG 2 contrast ratio, 1 to 21.
    fn contrast(a: Hsla, b: Hsla) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn contrast_formula() {
        let black = c(0x000000);
        let white = c(0xffffff);
        assert!((contrast(black, white) - 21.).abs() < 1e-6);
        assert!((contrast(white, white) - 1.).abs() < 1e-6);
        assert_eq!(contrast(black, white), contrast(white, black));
        // Known value: #777 on white is 4.48.
        assert!((contrast(c(0x777777), white) - 4.48).abs() < 0.01);
    }

    #[test]
    fn hex_round_trip() {
        // The palette is written as hex; going through Hsla must not drift.
        assert_eq!(hex(Theme::dark().bg), 0x282c34);
        assert_eq!(hex(Theme::light().fg), 0x242529);
        assert_eq!(hex(Theme::light().syntax.constant), 0xc18401);
    }

    /// WCAG AA for normal text.
    const TEXT: f64 = 4.5;
    /// De-emphasized text (muted, callout titles) and non-text UI (checkbox):
    /// WCAG AA for large text and for UI components.
    const SECONDARY: f64 = 3.0;

    /// One contrast requirement: `fg` drawn on `bg` should reach `goal`.
    /// `pinned` is set, with a TODO, where the current palette falls short:
    /// the test then only guards against getting worse. Changing the colors
    /// is a design decision, so it isn't done here.
    struct Check {
        what: String,
        fg: Hsla,
        bg: Hsla,
        goal: f64,
        pinned: Option<f64>,
    }

    fn checks(name: &str, t: &Theme) -> Vec<Check> {
        let light = !t.dark;
        let mut out = Vec::new();
        let mut add = |what: &str, fg, bg, goal, pinned: Option<f64>| {
            out.push(Check {
                what: format!("{name}: {what}"),
                fg,
                bg,
                goal,
                pinned,
            })
        };
        // Body text, everywhere it's drawn.
        add("text on bg", t.fg, t.bg, TEXT, None);
        add(
            "text on code_bg (inline code, table header)",
            t.fg,
            t.code_bg,
            TEXT,
            None,
        );
        add("text on bar_bg", t.fg, t.bar_bg, TEXT, None);
        add(
            "text on hover (tab close button)",
            t.fg,
            t.hover,
            TEXT,
            None,
        );
        // Links.
        // TODO(contrast): light link #5c78e2 is 3.84 on bg and 3.49 on code_bg.
        add("link on bg", t.link, t.bg, TEXT, light.then_some(3.8));
        add(
            "link on code_bg (table header)",
            t.link,
            t.code_bg,
            TEXT,
            light.then_some(3.45),
        );
        // Error text of a math/diagram block.
        // TODO(contrast): caution on code_bg is 3.78 (dark) / 3.29 (light).
        add(
            "caution on code_bg (render error)",
            t.caution,
            t.code_bg,
            TEXT,
            Some(if light { 3.25 } else { 3.75 }),
        );
        // Code: every token color on the code background.
        for tok in ALL_TOKS {
            // TODO(contrast): these token colors (from Zed's One Light/Dark)
            // are below 4.5 on code_bg. Current values are pinned.
            let pinned = match (light, tok) {
                (false, Tok::Keyword) => Some(3.8),
                (false, Tok::Comment) => Some(3.75),
                (false, Tok::Variable | Tok::Property | Tok::Deleted) => Some(3.75),
                (false, Tok::Meta) => Some(3.6),
                (true, Tok::Keyword) => Some(4.4),
                (true, Tok::Type | Tok::Number) => Some(3.6),
                (true, Tok::Function) => Some(3.45),
                (true, Tok::Attr | Tok::Tag) => Some(3.45),
                (true, Tok::Variable | Tok::Property | Tok::Deleted) => Some(3.25),
                (true, Tok::Meta) => Some(3.4),
                (true, Tok::Comment) => Some(2.8),
                (true, Tok::String | Tok::Constant | Tok::Inserted) => Some(2.7),
                _ => None,
            };
            add(
                &format!("{tok:?} on code_bg"),
                t.token(tok),
                t.code_bg,
                TEXT,
                pinned,
            );
        }
        // Muted text: lang labels, list markers, footnotes, quotes, H6,
        // inactive tabs.
        add("muted on bg", t.muted, t.bg, SECONDARY, None);
        add("muted on code_bg", t.muted, t.code_bg, SECONDARY, None);
        add(
            "muted on bar_bg (inactive tab)",
            t.muted,
            t.bar_bg,
            SECONDARY,
            None,
        );
        // TODO(contrast): light muted on hover is 2.96.
        add(
            "muted on hover (hovered tab's close button)",
            t.muted,
            t.hover,
            SECONDARY,
            light.then_some(2.95),
        );
        // Callout titles ("Note", "Warning", …), semibold.
        for k in ALL_CALLOUTS {
            add(
                &format!("{k:?} title on bg"),
                t.callout(k),
                t.bg,
                SECONDARY,
                None,
            );
        }
        // Checkbox: the check mark is drawn in bg on accent; the empty box
        // has a muted border.
        add("checkmark (bg on accent)", t.bg, t.accent, SECONDARY, None);
        add("empty checkbox border", t.muted, t.bg, SECONDARY, None);
        out
    }

    #[test]
    fn contrast_ratios() {
        let mut failures = Vec::new();
        let mut report = String::new();
        for (name, t) in themes() {
            for ch in checks(name, &t) {
                let ratio = contrast(ch.fg, ch.bg);
                report += &format!("{:5.2} (goal {}) {}\n", ratio, ch.goal, ch.what);
                match ch.pinned {
                    None if ratio < ch.goal => {
                        failures.push(format!("{}: {ratio:.2} < {}", ch.what, ch.goal))
                    }
                    Some(min) if ratio < min => failures.push(format!(
                        "{}: {ratio:.2} got worse than the pinned {min}",
                        ch.what
                    )),
                    Some(min) if ratio >= ch.goal => failures.push(format!(
                        "{}: {ratio:.2} now meets the goal {}; remove the pinned {min} and its TODO",
                        ch.what, ch.goal
                    )),
                    Some(min) => assert!(min < ch.goal),
                    None => {}
                }
            }
        }
        println!("{report}");
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn decorative_colors_are_visible() {
        for (name, t) in themes() {
            // Surfaces must differ from the page so code blocks, the tab bar
            // and hover states are visible at all.
            for (what, c) in [
                ("border", t.border),
                ("code_bg", t.code_bg),
                ("bar_bg", t.bar_bg),
                ("hover", t.hover),
            ] {
                assert!(contrast(c, t.bg) > 1.03, "{name}: {what} vs bg");
            }
            assert!(contrast(t.border, t.code_bg) > 1.03, "{name}: table border");
            assert!(contrast(t.hover, t.bar_bg) > 1.03, "{name}: tab hover");
        }
    }

    #[test]
    fn appearance_mapping() {
        use WindowAppearance::*;
        for (a, dark) in [
            (Light, false),
            (VibrantLight, false),
            (Dark, true),
            (VibrantDark, true),
        ] {
            let t = Theme::for_appearance(a);
            assert_eq!(t.dark, dark, "{a:?}");
            let want = if dark { Theme::dark() } else { Theme::light() };
            assert_eq!(hex(t.bg), hex(want.bg), "{a:?}");
            assert_eq!(hex(t.fg), hex(want.fg), "{a:?}");
        }
        assert!(Theme::dark().dark);
        assert!(!Theme::light().dark);
        // Dark really is dark.
        assert!(luminance(Theme::dark().bg) < luminance(Theme::dark().fg));
        assert!(luminance(Theme::light().bg) > luminance(Theme::light().fg));
    }

    #[test]
    fn token_mapping() {
        for (name, t) in themes() {
            let s = &t.syntax;
            let expect = [
                (Tok::Keyword, s.keyword),
                (Tok::Type, s.ty),
                (Tok::Function, s.function),
                (Tok::String, s.string),
                (Tok::Number, s.number),
                (Tok::Comment, s.comment),
                (Tok::Constant, s.constant),
                (Tok::Attr, s.attr),
                (Tok::Variable, s.variable),
                (Tok::Tag, s.tag),
                (Tok::Property, s.property),
                (Tok::Inserted, t.tip),
                (Tok::Deleted, t.caution),
                (Tok::Meta, t.muted),
            ];
            assert_eq!(expect.len(), ALL_TOKS.len());
            for (tok, color) in expect {
                assert_eq!(hex(t.token(tok)), hex(color), "{name}: {tok:?}");
            }
            // Every token stands out from plain code text and from the
            // background.
            for tok in ALL_TOKS {
                assert_ne!(hex(t.token(tok)), hex(t.fg), "{name}: {tok:?} vs fg");
                assert_ne!(
                    hex(t.token(tok)),
                    hex(t.code_bg),
                    "{name}: {tok:?} vs code_bg"
                );
            }
            // These kinds must be told apart from each other. (Attr, Tag and
            // Function share the link blue, Property shares Variable's red,
            // by design of the One themes.)
            let distinct = [
                Tok::Keyword,
                Tok::Type,
                Tok::Function,
                Tok::String,
                Tok::Number,
                Tok::Comment,
                Tok::Constant,
                Tok::Variable,
            ];
            for (i, a) in distinct.iter().enumerate() {
                for b in &distinct[i + 1..] {
                    assert_ne!(hex(t.token(*a)), hex(t.token(*b)), "{name}: {a:?} vs {b:?}");
                }
            }
            assert_ne!(hex(t.token(Tok::Inserted)), hex(t.token(Tok::Deleted)));
        }
    }

    #[test]
    fn callout_mapping() {
        for (name, t) in themes() {
            let expect = [
                (Callout::Note, t.note),
                (Callout::Tip, t.tip),
                (Callout::Important, t.important),
                (Callout::Warning, t.warning),
                (Callout::Caution, t.caution),
            ];
            for (k, color) in expect {
                assert_eq!(hex(t.callout(k)), hex(color), "{name}: {k:?}");
            }
            for (i, a) in ALL_CALLOUTS.iter().enumerate() {
                assert_ne!(
                    hex(t.callout(*a)),
                    hex(t.border),
                    "{name}: {a:?} vs plain quote"
                );
                for b in &ALL_CALLOUTS[i + 1..] {
                    assert_ne!(
                        hex(t.callout(*a)),
                        hex(t.callout(*b)),
                        "{name}: {a:?} vs {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn light_and_dark_differ_everywhere() {
        let (l, d) = (Theme::light(), Theme::dark());
        for (what, a, b) in [
            ("bg", l.bg, d.bg),
            ("fg", l.fg, d.fg),
            ("muted", l.muted, d.muted),
            ("border", l.border, d.border),
            ("link", l.link, d.link),
            ("code_bg", l.code_bg, d.code_bg),
            ("bar_bg", l.bar_bg, d.bar_bg),
            ("hover", l.hover, d.hover),
        ] {
            assert_ne!(hex(a), hex(b), "{what}");
        }
        for tok in ALL_TOKS {
            assert_ne!(hex(l.token(tok)), hex(d.token(tok)), "{tok:?}");
        }
    }
}
