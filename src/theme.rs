use gpui::{Hsla, WindowAppearance, rgb};

use crate::markdown::Callout;

#[derive(Clone, Copy)]
pub struct Theme {
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
        }
    }

    pub fn light() -> Self {
        Self {
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
}
