//! LaTeX math → Unicode text, for inline `$…$` math.
//!
//! GPUI lays out a paragraph as one run of text and can't place an image
//! inside it, so inline math is turned into text instead: Greek letters and
//! symbols become their Unicode characters, `x^2` becomes `x²`, `\frac{a}{b}`
//! becomes `a/b`, and variables are set in italics. Display math (`$$…$$`)
//! is typeset for real (see `graphics.rs`). Must not depend on GPUI.

use std::ops::Range;

/// Text plus the byte ranges that should be italic (variables).
#[derive(Debug, Default, PartialEq)]
pub struct MathText {
    pub text: String,
    pub italic: Vec<Range<usize>>,
}

pub fn to_unicode(tex: &str) -> MathText {
    let toks = lex(tex);
    let mut p = Parser { t: &toks, i: 0 };
    let mut w = W::default();
    p.seq(&mut w, Stop::Eof);
    let mut out = MathText {
        text: w.s.trim().to_string(),
        italic: Vec::new(),
    };
    // Re-base the italic ranges after trimming.
    let lead = w.s.len() - w.s.trim_start().len();
    for r in w.italic {
        let (s, e) = (r.start.saturating_sub(lead), r.end.saturating_sub(lead));
        let e = e.min(out.text.len());
        if s < e {
            out.italic.push(s..e);
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
enum T {
    Cmd(String),
    Ch(char),
    Open,
    Close,
    Sup,
    Sub,
    Amp,
}

fn lex(s: &str) -> Vec<T> {
    let mut out = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.peek() {
                Some(&n) if n.is_ascii_alphabetic() => {
                    let mut name = String::new();
                    while let Some(&n) = it.peek() {
                        if !n.is_ascii_alphabetic() {
                            break;
                        }
                        name.push(n);
                        it.next();
                    }
                    // `\operatorname*`
                    if it.peek() == Some(&'*') {
                        it.next();
                    }
                    out.push(T::Cmd(name));
                }
                Some(&n) => {
                    it.next();
                    out.push(T::Cmd(n.to_string()));
                }
                None => {}
            },
            '{' => out.push(T::Open),
            '}' => out.push(T::Close),
            '^' => out.push(T::Sup),
            '_' => out.push(T::Sub),
            '&' => out.push(T::Amp),
            '%' => {
                // Comment to end of line.
                for n in it.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
            }
            c => out.push(T::Ch(c)),
        }
    }
    out
}

/// Spacing class of the last thing written, as in TeX.
#[derive(Clone, Copy, PartialEq)]
enum Class {
    /// Start of a (sub)formula, or after an opening delimiter.
    Start,
    Ord,
    Bin,
    Rel,
    Punct,
}

#[derive(Default)]
struct W {
    s: String,
    italic: Vec<Range<usize>>,
    /// Inside a sub/superscript: no spacing around operators.
    script: bool,
    upright: bool,
    last: Option<Class>,
}

impl W {
    fn sub(&self) -> W {
        W {
            script: self.script,
            upright: self.upright,
            ..W::default()
        }
    }

    fn ord(&mut self, s: &str, italic: bool) {
        let start = self.s.len();
        self.s.push_str(s);
        if italic && !self.upright {
            match self.italic.last_mut() {
                Some(r) if r.end == start => r.end = self.s.len(),
                _ => self.italic.push(start..self.s.len()),
            }
        }
        self.last = Some(Class::Ord);
    }

    fn sym(&mut self, s: &str, class: Class) {
        let prev = self.last.unwrap_or(Class::Start);
        match class {
            Class::Bin if prev == Class::Ord && !self.script => {
                self.s.push(' ');
                self.s.push_str(s);
                self.s.push(' ');
            }
            // A binary operator after nothing/an operator is unary: `-x`.
            Class::Bin => {
                self.s.push_str(s);
                self.last = Some(Class::Start);
                return;
            }
            Class::Rel if !self.script => {
                if !self.s.ends_with(' ') && !self.s.is_empty() {
                    self.s.push(' ');
                }
                self.s.push_str(s);
                self.s.push(' ');
            }
            Class::Punct if !self.script => {
                self.s.push_str(s);
                self.s.push('\u{2009}');
            }
            _ => self.s.push_str(s),
        }
        self.last = Some(match class {
            Class::Start => Class::Start,
            c => c,
        });
    }

    fn append(&mut self, other: W) {
        let base = self.s.len();
        self.s.push_str(&other.s);
        self.italic.extend(
            other
                .italic
                .into_iter()
                .map(|r| r.start + base..r.end + base),
        );
        if other.last.is_some() {
            self.last = Some(Class::Ord);
        }
    }

    fn space(&mut self, s: &str) {
        self.s.push_str(s);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Stop {
    Eof,
    Close,
    /// `\end{…}`.
    End,
    /// `]` (for `\sqrt[n]`).
    Bracket,
}

struct Parser<'a> {
    t: &'a [T],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&T> {
        self.t.get(self.i)
    }

    fn next(&mut self) -> Option<T> {
        let t = self.t.get(self.i).cloned();
        self.i += 1;
        t
    }

    fn seq(&mut self, w: &mut W, stop: Stop) {
        while let Some(t) = self.peek() {
            match (t, stop) {
                (T::Close, Stop::Close) => {
                    self.i += 1;
                    return;
                }
                (T::Close, _) => {
                    self.i += 1;
                    continue;
                }
                (T::Ch(']'), Stop::Bracket) => {
                    self.i += 1;
                    return;
                }
                (T::Cmd(c), Stop::End) if c == "end" => return,
                _ => {}
            }
            self.item(w);
        }
    }

    /// One argument: a `{group}` or a single token.
    fn arg(&mut self, w: &W) -> W {
        let mut out = w.sub();
        self.skip_spaces();
        match self.peek() {
            Some(T::Open) => {
                self.i += 1;
                self.seq(&mut out, Stop::Close);
            }
            Some(_) => self.item(&mut out),
            None => {}
        }
        out
    }

    /// Raw text of a `{group}` (for `\text`, `\begin{env}`...).
    fn raw_arg(&mut self) -> String {
        self.skip_spaces();
        let mut s = String::new();
        if self.peek() != Some(&T::Open) {
            if let Some(T::Ch(c)) = self.next() {
                s.push(c);
            }
            return s;
        }
        self.i += 1;
        // Inner braces only group, as in TeX: `\text{a {b}}` is "a b".
        let mut depth = 0;
        while let Some(t) = self.next() {
            match t {
                T::Open => depth += 1,
                T::Close if depth == 0 => break,
                T::Close => depth -= 1,
                T::Ch(c) => s.push(c),
                T::Cmd(c) => match symbol(&c) {
                    Some((sym, _)) => s.push_str(sym),
                    None if c.len() == 1 => s.push_str(&c),
                    None => {}
                },
                T::Sup => s.push('^'),
                T::Sub => s.push('_'),
                T::Amp => s.push('&'),
            }
        }
        s
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(T::Ch(c)) if c.is_whitespace()) {
            self.i += 1;
        }
    }

    fn optional_arg(&mut self, w: &W) -> Option<W> {
        if self.peek() == Some(&T::Ch('[')) {
            self.i += 1;
            let mut out = w.sub();
            self.seq(&mut out, Stop::Bracket);
            Some(out)
        } else {
            None
        }
    }

    fn item(&mut self, w: &mut W) {
        let Some(t) = self.next() else { return };
        match t {
            T::Open => {
                let mut g = w.sub();
                self.seq(&mut g, Stop::Close);
                w.append(g);
            }
            T::Close => {}
            T::Sup | T::Sub => {
                let mut inner = w.sub();
                inner.script = true;
                let arg = {
                    self.skip_spaces();
                    match self.peek() {
                        Some(T::Open) => {
                            self.i += 1;
                            self.seq(&mut inner, Stop::Close);
                        }
                        Some(_) => self.item(&mut inner),
                        None => {}
                    }
                    inner
                };
                script(w, arg, t == T::Sup);
            }
            T::Amp => w.space("  "),
            T::Ch(c) => self.ch(w, c),
            T::Cmd(name) => self.cmd(w, &name),
        }
    }

    fn ch(&mut self, w: &mut W, c: char) {
        match c {
            c if c.is_whitespace() => {}
            '\'' => {
                // Primes: f', f''.
                let mut n = 1;
                while self.peek() == Some(&T::Ch('\'')) {
                    self.i += 1;
                    n += 1;
                }
                w.ord(["′", "″", "‴"][(n - 1).min(2)], false);
            }
            '~' => w.space("\u{a0}"),
            '-' => w.sym("−", Class::Bin),
            '+' => w.sym("+", Class::Bin),
            '*' => w.sym("∗", Class::Bin),
            '=' | '<' | '>' => w.sym(&c.to_string(), Class::Rel),
            ':' => w.sym(":", Class::Rel),
            ',' | ';' => w.sym(&c.to_string(), Class::Punct),
            '(' | '[' => {
                w.s.push(c);
                w.last = Some(Class::Start);
            }
            c if c.is_ascii_alphabetic() => w.ord(&c.to_string(), true),
            c => w.ord(&c.to_string(), false),
        }
    }

    fn cmd(&mut self, w: &mut W, name: &str) {
        if let Some((s, class)) = symbol(name) {
            match class {
                Kind::Ord => w.ord(s, false),
                Kind::Var => w.ord(s, true),
                Kind::Bin => w.sym(s, Class::Bin),
                Kind::Rel => w.sym(s, Class::Rel),
                Kind::Op => {
                    w.ord(s, false);
                }
                Kind::Open => {
                    w.s.push_str(s);
                    w.last = Some(Class::Start);
                }
            }
            return;
        }
        if is_function(name) {
            w.ord(name, false);
            self.thin_space_before_arg(w);
            return;
        }
        match name {
            // Spacing.
            "," | "thinspace" => w.space("\u{2009}"),
            ":" | ">" | "medspace" => w.space("\u{205f}"),
            ";" | "thickspace" => w.space("\u{2004}"),
            "quad" => w.space("\u{2003}"),
            "qquad" => w.space("\u{2003}\u{2003}"),
            " " => w.space(" "),
            "!" | "negthinspace" => {}
            "\\" | "newline" | "cr" => w.space("; "),
            // Ignored.
            "left" | "right" | "middle" | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr"
            | "Bigl" | "Bigr" | "biggl" | "biggr" | "Biggl" | "Biggr" | "displaystyle"
            | "textstyle" | "scriptstyle" | "scriptscriptstyle" | "limits" | "nolimits"
            | "nonumber" | "notag" | "mathstrut" | "strut" | "relax" | "allowbreak" => {
                // `\left.` is an invisible delimiter.
                if matches!(name, "left" | "right" | "middle") && self.peek() == Some(&T::Ch('.')) {
                    self.i += 1;
                }
            }
            "label" | "hspace" | "vspace" | "kern" | "mkern" | "hskip" | "phantom" => {
                self.raw_arg();
            }
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let a = self.arg(w);
                let b = self.arg(w);
                fraction(w, a, b);
            }
            "binom" | "dbinom" | "tbinom" => {
                let a = self.arg(w);
                let b = self.arg(w);
                w.s.push('(');
                w.append(a);
                w.s.push_str(" choose ");
                w.append(b);
                w.s.push(')');
                w.last = Some(Class::Ord);
            }
            "sqrt" => {
                let n = self.optional_arg(w);
                let a = self.arg(w);
                let root = match n.as_ref().map(|n| n.s.trim()) {
                    Some("3") => "∛".to_string(),
                    Some("4") => "∜".to_string(),
                    Some(n) => match to_script(n, true) {
                        Some(s) => format!("{s}√"),
                        None => format!("({n})√"),
                    },
                    None => "√".to_string(),
                };
                w.ord(&root, false);
                wrap(w, a);
            }
            "text" | "textrm" | "textnormal" | "textup" | "mbox" | "hbox" | "textsf" | "texttt"
            | "textmd" => {
                let s = self.raw_arg();
                w.ord(&s, false);
            }
            "textit" | "emph" | "textsl" => {
                let s = self.raw_arg();
                w.ord(&s, true);
            }
            "textbf" => {
                let s = self.raw_arg();
                w.ord(&s, false);
            }
            "mathrm" | "mathup" | "mathbf" | "mathsf" | "mathtt" | "boldsymbol" | "bm"
            | "mathnormal" | "operatorname" | "mathop" => {
                let mut a = w.sub();
                a.upright = name != "mathnormal" && name != "boldsymbol" && name != "bm";
                let mut g = self.arg(&a);
                g.italic.clear();
                if name == "boldsymbol" || name == "bm" || name == "mathnormal" {
                    g.italic.push(0..g.s.len());
                }
                w.append(g);
                if matches!(name, "operatorname" | "mathop") {
                    self.thin_space_before_arg(w);
                }
            }
            "mathit" => {
                // The whole group is italic: replace (not add to) the ranges
                // it already has, or they would overlap.
                let mut g = self.arg(w);
                g.italic.clear();
                if !g.s.is_empty() {
                    g.italic.push(0..g.s.len());
                }
                w.append(g);
            }
            "mathbb" | "Bbb" | "mathcal" | "mathscr" | "mathfrak" => {
                let s = self.raw_arg();
                let mapped: String = s.chars().map(|c| alphabet(name, c)).collect();
                w.ord(&mapped, false);
            }
            // The group's own operator spacing is replaced by the class's.
            "mathbin" => {
                let g = self.arg(w);
                w.sym(g.s.trim(), Class::Bin);
            }
            "mathrel" => {
                let g = self.arg(w);
                w.sym(g.s.trim(), Class::Rel);
            }
            "mathord" | "mathopen" | "mathclose" | "mathpunct" | "mathinner" | "underbrace"
            | "overbrace" | "boxed" | "fbox" | "smash" | "cancel" | "bcancel" | "xcancel"
            | "sout" => {
                let g = self.arg(w);
                w.append(g);
            }
            "color" => {
                self.raw_arg();
            }
            "textcolor" | "colorbox" => {
                self.raw_arg();
                let g = self.arg(w);
                w.append(g);
            }
            "overset" | "stackrel" | "underset" => {
                let top = self.arg(w);
                let base = self.arg(w);
                w.append(base);
                script(w, top, name != "underset");
            }
            "not" => {
                // `\not=` → ≠, `\not\in` → ∉, otherwise a combining slash.
                let mut g = w.sub();
                self.item(&mut g);
                let s = match g.s.trim() {
                    "=" => "≠".to_string(),
                    "∈" => "∉".to_string(),
                    "⊂" => "⊄".to_string(),
                    "⊆" => "⊈".to_string(),
                    "≡" => "≢".to_string(),
                    "∼" => "≁".to_string(),
                    other => format!("{other}\u{338}"),
                };
                w.sym(&s, Class::Rel);
            }
            "tag" => {
                let s = self.raw_arg();
                w.space(&format!("\u{2003}({s})"));
            }
            "begin" => self.environment(w),
            "end" => {
                self.raw_arg();
            }
            _ => {
                if let Some(mark) = accent(name) {
                    let g = self.arg(w);
                    add_accent(w, g, mark, name);
                } else {
                    // Unknown command: show it as written.
                    w.ord(&format!("\\{name}"), false);
                }
            }
        }
    }

    /// `\sin x` gets a thin space; `\sin(x)` and `\sin^2` don't.
    fn thin_space_before_arg(&mut self, w: &mut W) {
        let mut j = self.i;
        while matches!(self.t.get(j), Some(T::Ch(c)) if c.is_whitespace()) {
            j += 1;
        }
        match self.t.get(j) {
            Some(T::Ch('(')) | Some(T::Sup) | Some(T::Sub) | None => {}
            Some(T::Cmd(c)) if c == "left" || c == "limits" => {}
            _ => w.space("\u{2009}"),
        }
    }

    fn environment(&mut self, w: &mut W) {
        let env = self.raw_arg();
        let env = env.trim_end_matches('*');
        if env == "array" || env == "tabular" {
            self.raw_arg(); // column spec
        }
        let (open, close) = match env {
            "pmatrix" => ("(", ")"),
            "bmatrix" => ("[", "]"),
            "Bmatrix" => ("{", "}"),
            "vmatrix" => ("|", "|"),
            "Vmatrix" => ("‖", "‖"),
            "cases" | "dcases" => ("{ ", ""),
            _ => ("", ""),
        };
        let mut g = w.sub();
        self.seq(&mut g, Stop::End);
        // Consume `\end{env}`.
        if matches!(self.peek(), Some(T::Cmd(c)) if c == "end") {
            self.i += 1;
            self.raw_arg();
        }
        let body = W {
            s: g.s
                .trim_end_matches(['\u{2003}', ' ', ';'])
                .trim()
                .to_string(),
            italic: Vec::new(),
            ..w.sub()
        };
        // Italics are dropped for simplicity; the layout is approximate anyway.
        let _ = g.italic;
        w.ord(open, false);
        w.ord(&body.s, false);
        w.ord(close, false);
    }
}

fn script(w: &mut W, arg: W, sup: bool) {
    if arg.s.is_empty() {
        return;
    }
    if let Some(s) = to_script(&arg.s, sup) {
        w.s.push_str(&s);
        w.last = Some(Class::Ord);
        return;
    }
    w.s.push(if sup { '^' } else { '_' });
    let simple = arg.s.chars().count() == 1;
    if !simple {
        w.s.push('(');
    }
    w.append(arg);
    if !simple {
        w.s.push(')');
    }
    w.last = Some(Class::Ord);
}

fn fraction(w: &mut W, a: W, b: W) {
    let vulgar = match (a.s.as_str(), b.s.as_str()) {
        ("1", "2") => Some("½"),
        ("1", "3") => Some("⅓"),
        ("2", "3") => Some("⅔"),
        ("1", "4") => Some("¼"),
        ("3", "4") => Some("¾"),
        ("1", "5") => Some("⅕"),
        ("1", "6") => Some("⅙"),
        ("1", "8") => Some("⅛"),
        _ => None,
    };
    if let Some(v) = vulgar {
        w.ord(v, false);
        return;
    }
    wrap(w, a);
    w.s.push('/');
    wrap(w, b);
    w.last = Some(Class::Ord);
}

/// Appends `g`, in parentheses if it's more than one simple term.
fn wrap(w: &mut W, g: W) {
    let t = g.s.trim();
    let simple = t.chars().count() <= 1
        || !t.chars().any(|c| {
            c.is_whitespace() || matches!(c, '+' | '−' | '/' | '=' | '·' | '⋅' | '×' | '±' | ',')
        })
        || (t.starts_with('(') && t.ends_with(')'));
    if simple {
        w.append(g);
    } else {
        w.s.push('(');
        w.append(g);
        w.s.push(')');
        w.last = Some(Class::Ord);
    }
}

fn add_accent(w: &mut W, g: W, mark: char, name: &str) {
    let per_char = matches!(name, "overline" | "underline" | "bar" | "widebar");
    let mut s = String::new();
    let n = g.s.chars().count();
    for (i, c) in g.s.chars().enumerate() {
        s.push(c);
        if per_char || i + 1 == n {
            s.push(mark);
        }
    }
    let italic = !g.italic.is_empty();
    w.ord(&s, italic);
}

fn accent(name: &str) -> Option<char> {
    Some(match name {
        "hat" | "widehat" => '\u{302}',
        "tilde" | "widetilde" => '\u{303}',
        "bar" | "overline" | "widebar" => '\u{305}',
        "underline" => '\u{332}',
        "vec" | "overrightarrow" => '\u{20d7}',
        "overleftarrow" => '\u{20d6}',
        "dot" => '\u{307}',
        "ddot" => '\u{308}',
        "dddot" => '\u{20db}',
        "acute" => '\u{301}',
        "grave" => '\u{300}',
        "check" => '\u{30c}',
        "breve" => '\u{306}',
        "mathring" => '\u{30a}',
        _ => return None,
    })
}

fn to_script(s: &str, sup: bool) -> Option<String> {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if sup { superscript(c) } else { subscript(c) })
        .collect()
}

fn superscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' | '−' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'i' => 'ⁱ',
        'j' => 'ʲ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'n' => 'ⁿ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'z' => 'ᶻ',
        'A' => 'ᴬ',
        'B' => 'ᴮ',
        'D' => 'ᴰ',
        'E' => 'ᴱ',
        'G' => 'ᴳ',
        'H' => 'ᴴ',
        'I' => 'ᴵ',
        'J' => 'ᴶ',
        'K' => 'ᴷ',
        'L' => 'ᴸ',
        'M' => 'ᴹ',
        'N' => 'ᴺ',
        'O' => 'ᴼ',
        'P' => 'ᴾ',
        'R' => 'ᴿ',
        'T' => 'ᵀ',
        'U' => 'ᵁ',
        'V' => 'ⱽ',
        'W' => 'ᵂ',
        'α' => 'ᵅ',
        'β' => 'ᵝ',
        'γ' => 'ᵞ',
        'δ' => 'ᵟ',
        'θ' => 'ᶿ',
        'φ' => 'ᵠ',
        'χ' => 'ᵡ',
        '∗' | '*' => '*',
        '′' => '′',
        '″' => '″',
        '∘' => '°',
        _ => return None,
    })
}

fn subscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' | '−' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        'β' => 'ᵦ',
        'γ' => 'ᵧ',
        'ρ' => 'ᵨ',
        'φ' => 'ᵩ',
        'χ' => 'ᵪ',
        _ => return None,
    })
}

/// `\mathbb`, `\mathcal`, `\mathfrak` letters.
fn alphabet(style: &str, c: char) -> char {
    let special = match (style, c) {
        ("mathbb" | "Bbb", 'C') => Some('ℂ'),
        ("mathbb" | "Bbb", 'H') => Some('ℍ'),
        ("mathbb" | "Bbb", 'N') => Some('ℕ'),
        ("mathbb" | "Bbb", 'P') => Some('ℙ'),
        ("mathbb" | "Bbb", 'Q') => Some('ℚ'),
        ("mathbb" | "Bbb", 'R') => Some('ℝ'),
        ("mathbb" | "Bbb", 'Z') => Some('ℤ'),
        ("mathcal" | "mathscr", 'B') => Some('ℬ'),
        ("mathcal" | "mathscr", 'E') => Some('ℰ'),
        ("mathcal" | "mathscr", 'F') => Some('ℱ'),
        ("mathcal" | "mathscr", 'H') => Some('ℋ'),
        ("mathcal" | "mathscr", 'I') => Some('ℐ'),
        ("mathcal" | "mathscr", 'L') => Some('ℒ'),
        ("mathcal" | "mathscr", 'M') => Some('ℳ'),
        ("mathcal" | "mathscr", 'R') => Some('ℛ'),
        ("mathcal" | "mathscr", 'e') => Some('ℯ'),
        ("mathcal" | "mathscr", 'g') => Some('ℊ'),
        ("mathcal" | "mathscr", 'o') => Some('ℴ'),
        ("mathfrak", 'C') => Some('ℭ'),
        ("mathfrak", 'H') => Some('ℌ'),
        ("mathfrak", 'I') => Some('ℑ'),
        ("mathfrak", 'R') => Some('ℜ'),
        ("mathfrak", 'Z') => Some('ℨ'),
        _ => None,
    };
    if let Some(s) = special {
        return s;
    }
    let (upper, lower, digit) = match style {
        "mathbb" | "Bbb" => (0x1D538, 0x1D552, Some(0x1D7D8)),
        "mathcal" | "mathscr" => (0x1D49C, 0x1D4B6, None),
        _ => (0x1D504, 0x1D51E, None),
    };
    let code = match c {
        'A'..='Z' => upper + (c as u32 - 'A' as u32),
        'a'..='z' => lower + (c as u32 - 'a' as u32),
        '0'..='9' => match digit {
            Some(d) => d + (c as u32 - '0' as u32),
            None => return c,
        },
        _ => return c,
    };
    char::from_u32(code).unwrap_or(c)
}

fn is_function(name: &str) -> bool {
    matches!(
        name,
        "sin"
            | "cos"
            | "tan"
            | "cot"
            | "sec"
            | "csc"
            | "arcsin"
            | "arccos"
            | "arctan"
            | "sinh"
            | "cosh"
            | "tanh"
            | "coth"
            | "log"
            | "ln"
            | "lg"
            | "exp"
            | "lim"
            | "liminf"
            | "limsup"
            | "max"
            | "min"
            | "sup"
            | "inf"
            | "det"
            | "dim"
            | "ker"
            | "deg"
            | "gcd"
            | "lcm"
            | "arg"
            | "hom"
            | "Pr"
            | "mod"
            | "bmod"
            | "tr"
            | "Tr"
            | "rank"
            | "sgn"
            | "argmax"
            | "argmin"
    )
}

#[derive(Clone, Copy)]
enum Kind {
    Ord,
    /// Italic variable (lowercase Greek).
    Var,
    Bin,
    Rel,
    /// Large operators: ∑ ∫ ...
    Op,
    Open,
}

fn symbol(name: &str) -> Option<(&'static str, Kind)> {
    use Kind::*;
    Some(match name {
        // Greek, lowercase (italic like other variables).
        "alpha" => ("α", Var),
        "beta" => ("β", Var),
        "gamma" => ("γ", Var),
        "delta" => ("δ", Var),
        "epsilon" => ("ϵ", Var),
        "varepsilon" => ("ε", Var),
        "zeta" => ("ζ", Var),
        "eta" => ("η", Var),
        "theta" => ("θ", Var),
        "vartheta" => ("ϑ", Var),
        "iota" => ("ι", Var),
        "kappa" => ("κ", Var),
        "lambda" => ("λ", Var),
        "mu" => ("μ", Var),
        "nu" => ("ν", Var),
        "xi" => ("ξ", Var),
        "omicron" => ("ο", Var),
        "pi" => ("π", Var),
        "varpi" => ("ϖ", Var),
        "rho" => ("ρ", Var),
        "varrho" => ("ϱ", Var),
        "sigma" => ("σ", Var),
        "varsigma" => ("ς", Var),
        "tau" => ("τ", Var),
        "upsilon" => ("υ", Var),
        "phi" => ("ϕ", Var),
        "varphi" => ("φ", Var),
        "chi" => ("χ", Var),
        "psi" => ("ψ", Var),
        "omega" => ("ω", Var),
        // Greek, uppercase (upright).
        "Gamma" => ("Γ", Ord),
        "Delta" => ("Δ", Ord),
        "Theta" => ("Θ", Ord),
        "Lambda" => ("Λ", Ord),
        "Xi" => ("Ξ", Ord),
        "Pi" => ("Π", Ord),
        "Sigma" => ("Σ", Ord),
        "Upsilon" => ("Υ", Ord),
        "Phi" => ("Φ", Ord),
        "Psi" => ("Ψ", Ord),
        "Omega" => ("Ω", Ord),
        // Binary operators.
        "pm" => ("±", Bin),
        "mp" => ("∓", Bin),
        "times" => ("×", Bin),
        "div" => ("÷", Bin),
        "cdot" => ("⋅", Bin),
        "ast" => ("∗", Bin),
        "star" => ("⋆", Bin),
        "circ" => ("∘", Bin),
        "bullet" => ("∙", Bin),
        "oplus" => ("⊕", Bin),
        "ominus" => ("⊖", Bin),
        "otimes" => ("⊗", Bin),
        "odot" => ("⊙", Bin),
        "cup" => ("∪", Bin),
        "cap" => ("∩", Bin),
        "sqcup" => ("⊔", Bin),
        "sqcap" => ("⊓", Bin),
        "wedge" | "land" => ("∧", Bin),
        "vee" | "lor" => ("∨", Bin),
        "setminus" => ("∖", Bin),
        "dagger" => ("†", Bin),
        "ddagger" => ("‡", Bin),
        "amalg" => ("⨿", Bin),
        // Relations.
        "le" | "leq" => ("≤", Rel),
        "ge" | "geq" => ("≥", Rel),
        "leqslant" => ("⩽", Rel),
        "geqslant" => ("⩾", Rel),
        "ne" | "neq" => ("≠", Rel),
        "approx" => ("≈", Rel),
        "equiv" => ("≡", Rel),
        "sim" => ("∼", Rel),
        "simeq" => ("≃", Rel),
        "cong" => ("≅", Rel),
        "propto" => ("∝", Rel),
        "ll" => ("≪", Rel),
        "gg" => ("≫", Rel),
        "prec" => ("≺", Rel),
        "succ" => ("≻", Rel),
        "preceq" => ("⪯", Rel),
        "succeq" => ("⪰", Rel),
        "subset" => ("⊂", Rel),
        "supset" => ("⊃", Rel),
        "subseteq" => ("⊆", Rel),
        "supseteq" => ("⊇", Rel),
        "subsetneq" => ("⊊", Rel),
        "supsetneq" => ("⊋", Rel),
        "in" => ("∈", Rel),
        "notin" => ("∉", Rel),
        "ni" => ("∋", Rel),
        "mid" => ("∣", Rel),
        "nmid" => ("∤", Rel),
        "parallel" => ("∥", Rel),
        "perp" => ("⊥", Rel),
        "vdash" => ("⊢", Rel),
        "dashv" => ("⊣", Rel),
        "models" => ("⊨", Rel),
        "coloneqq" | "coloneq" => ("≔", Rel),
        "doteq" => ("≐", Rel),
        "triangleq" => ("≜", Rel),
        "to" | "rightarrow" => ("→", Rel),
        "gets" | "leftarrow" => ("←", Rel),
        "leftrightarrow" => ("↔", Rel),
        "Rightarrow" => ("⇒", Rel),
        "Leftarrow" => ("⇐", Rel),
        "Leftrightarrow" => ("⇔", Rel),
        "implies" | "Longrightarrow" => ("⟹", Rel),
        "impliedby" | "Longleftarrow" => ("⟸", Rel),
        "iff" | "Longleftrightarrow" => ("⟺", Rel),
        "longrightarrow" => ("⟶", Rel),
        "longleftarrow" => ("⟵", Rel),
        "longleftrightarrow" => ("⟷", Rel),
        "mapsto" => ("↦", Rel),
        "longmapsto" => ("⟼", Rel),
        "hookrightarrow" => ("↪", Rel),
        "hookleftarrow" => ("↩", Rel),
        "uparrow" => ("↑", Rel),
        "downarrow" => ("↓", Rel),
        "updownarrow" => ("↕", Rel),
        "Uparrow" => ("⇑", Rel),
        "Downarrow" => ("⇓", Rel),
        "nearrow" => ("↗", Rel),
        "searrow" => ("↘", Rel),
        "rightleftharpoons" => ("⇌", Rel),
        // Large operators.
        "sum" => ("∑", Op),
        "prod" => ("∏", Op),
        "coprod" => ("∐", Op),
        "int" => ("∫", Op),
        "iint" => ("∬", Op),
        "iiint" => ("∭", Op),
        "oint" => ("∮", Op),
        "bigcup" => ("⋃", Op),
        "bigcap" => ("⋂", Op),
        "bigoplus" => ("⨁", Op),
        "bigotimes" => ("⨂", Op),
        "bigvee" => ("⋁", Op),
        "bigwedge" => ("⋀", Op),
        "bigsqcup" => ("⨆", Op),
        // Delimiters.
        "langle" => ("⟨", Open),
        "rangle" => ("⟩", Ord),
        "lceil" => ("⌈", Open),
        "rceil" => ("⌉", Ord),
        "lfloor" => ("⌊", Open),
        "rfloor" => ("⌋", Ord),
        "lvert" => ("|", Open),
        "rvert" => ("|", Ord),
        "lVert" => ("‖", Open),
        "rVert" => ("‖", Ord),
        "vert" => ("|", Ord),
        "Vert" | "|" => ("‖", Ord),
        "{" | "lbrace" => ("{", Open),
        "}" | "rbrace" => ("}", Ord),
        "lbrack" => ("[", Open),
        "rbrack" => ("]", Ord),
        "backslash" => ("\\", Ord),
        // Other symbols.
        "infty" => ("∞", Ord),
        "partial" => ("∂", Ord),
        "nabla" => ("∇", Ord),
        "forall" => ("∀", Ord),
        "exists" => ("∃", Ord),
        "nexists" => ("∄", Ord),
        "emptyset" | "varnothing" => ("∅", Ord),
        "neg" | "lnot" => ("¬", Ord),
        "ldots" | "dots" | "dotsc" | "dotsb" => ("…", Ord),
        "cdots" => ("⋯", Ord),
        "vdots" => ("⋮", Ord),
        "ddots" => ("⋱", Ord),
        "prime" => ("′", Ord),
        "angle" => ("∠", Ord),
        "measuredangle" => ("∡", Ord),
        "triangle" => ("△", Ord),
        "square" | "Box" => ("□", Ord),
        "blacksquare" | "qed" => ("■", Ord),
        "Diamond" => ("◇", Ord),
        "hbar" | "hslash" => ("ℏ", Var),
        "ell" => ("ℓ", Var),
        "wp" => ("℘", Ord),
        "Re" => ("ℜ", Ord),
        "Im" => ("ℑ", Ord),
        "aleph" => ("ℵ", Ord),
        "beth" => ("ℶ", Ord),
        "top" => ("⊤", Ord),
        "bot" => ("⊥", Ord),
        "therefore" => ("∴", Ord),
        "because" => ("∵", Ord),
        "checkmark" => ("✓", Ord),
        "degree" => ("°", Ord),
        "flat" => ("♭", Ord),
        "sharp" => ("♯", Ord),
        "natural" => ("♮", Ord),
        "clubsuit" => ("♣", Ord),
        "diamondsuit" => ("♢", Ord),
        "heartsuit" => ("♡", Ord),
        "spadesuit" => ("♠", Ord),
        "imath" => ("ı", Var),
        "jmath" => ("ȷ", Var),
        "S" => ("§", Ord),
        "P" => ("¶", Ord),
        "%" => ("%", Ord),
        "$" => ("$", Ord),
        "&" => ("&", Ord),
        "#" => ("#", Ord),
        "_" => ("_", Ord),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> String {
        to_unicode(s).text
    }

    #[test]
    fn converts_common_math() {
        assert_eq!(u("x^2 + y^2 = z^2"), "x² + y² = z²");
        assert_eq!(u(r"\alpha_i \le \beta_{ij}"), "αᵢ ≤ βᵢⱼ");
        assert_eq!(u(r"e^{i\pi} + 1 = 0"), "e^(iπ) + 1 = 0");
        assert_eq!(u(r"\frac{1}{2}"), "½");
        assert_eq!(u(r"\frac{a+b}{c}"), "(a + b)/c");
        assert_eq!(u(r"\sqrt{x}"), "√x");
        assert_eq!(u(r"\sqrt[3]{x+1}"), "∛(x + 1)");
        assert_eq!(u(r"\mathbb{R}^n"), "ℝⁿ");
        assert_eq!(u(r"\sin x"), "sin\u{2009}x");
        assert_eq!(u(r"\sin(x)"), "sin(x)");
        assert_eq!(u(r"-x"), "−x");
        assert_eq!(u(r"f'(x)"), "f′(x)");
        assert_eq!(u(r"\text{if } x"), "if x");
        assert_eq!(u(r"\hat{x}"), "x\u{302}");
        assert_eq!(u(r"a \not= b"), "a ≠ b");
        assert_eq!(u(r"\left( x \right)"), "(x)");
        assert_eq!(u(r"\sum_{i=1}^n i"), "∑ᵢ₌₁ⁿi");
        assert_eq!(u(r"\foo"), "\\foo");
        assert_eq!(u(r"f(x, y)"), "f(x,\u{2009}y)");
    }

    #[test]
    fn mathbin_and_mathrel_space_once() {
        assert_eq!(u(r"a \mathrel{=} b \mathbin{+} c"), "a = b + c");
        assert_eq!(u(r"a \mathrel{\sim} b"), "a ∼ b");
        assert_eq!(u(r"\mathbin{\star} x"), "⋆x");
    }

    #[test]
    fn braces_inside_text_only_group() {
        assert_eq!(u(r"\text{a {nested} \{b\} c}"), "a nested {b} c");
        assert_eq!(u(r"\mathbb{{R}}"), "ℝ");
        assert_eq!(u(r"\text{a {b"), "a b");
    }

    #[test]
    fn italic_ranges_cover_variables() {
        let m = to_unicode(r"x + \sin y");
        let it: Vec<&str> = m.italic.iter().map(|r| &m.text[r.clone()]).collect();
        assert_eq!(it, ["x", "y"]);
        // Valid char boundaries even with multi-byte characters.
        let m = to_unicode(r"\alpha\beta + \mathrm{d}x");
        for r in &m.italic {
            assert!(m.text.is_char_boundary(r.start) && m.text.is_char_boundary(r.end));
        }
    }

    #[test]
    fn mathit_italic_ranges_do_not_overlap() {
        // `\mathit{x}` used to report `x` as italic twice ([0..1, 0..1]),
        // which made `RichText::push_math` slice backwards and panic.
        let m = to_unicode(r"\mathit{xy} + \mathit{\alpha 1}");
        assert_eq!(m.text, "xy + α1");
        assert_eq!(m.italic, [0..2, 5..8]);
    }
}
