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
