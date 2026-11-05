//! LaTeX → Unicode (`latex::to_unicode`, used for inline math): a reviewed
//! snapshot table of every symbol, function, accent and structural command,
//! in context and malformed, plus targeted assertions. Every conversion is
//! also checked against the `MathText` invariants.
//!
//! Snapshot lines read `input ⇒ output`, with italic ranges as `⟦I|…⟧` and
//! invisible characters as `⟨U+XXXX⟩` (see `common/dump.rs`).

mod common;

use std::collections::BTreeSet;

use common::{dump, invariants::to_unicode_and_check};
use mdview_core::markdown::{ITALIC, RichText, Span};

/// Every name `symbol()` knows. `symbol_list_matches_the_source` keeps this
/// in sync with `src/latex.rs`.
const SYMBOLS: &[&str] = &[
    "alpha",
    "beta",
    "gamma",
    "delta",
    "epsilon",
    "varepsilon",
    "zeta",
    "eta",
    "theta",
    "vartheta",
    "iota",
    "kappa",
    "lambda",
    "mu",
    "nu",
    "xi",
    "omicron",
    "pi",
    "varpi",
    "rho",
    "varrho",
    "sigma",
    "varsigma",
    "tau",
    "upsilon",
    "phi",
    "varphi",
    "chi",
    "psi",
    "omega",
    "Gamma",
    "Delta",
    "Theta",
    "Lambda",
    "Xi",
    "Pi",
    "Sigma",
    "Upsilon",
    "Phi",
    "Psi",
    "Omega",
    "pm",
    "mp",
    "times",
    "div",
    "cdot",
    "ast",
    "star",
    "circ",
    "bullet",
    "oplus",
    "ominus",
    "otimes",
    "odot",
    "cup",
    "cap",
    "sqcup",
    "sqcap",
    "wedge",
    "land",
    "vee",
    "lor",
    "setminus",
    "dagger",
    "ddagger",
    "amalg",
    "le",
    "leq",
    "ge",
    "geq",
    "leqslant",
    "geqslant",
    "ne",
    "neq",
    "approx",
    "equiv",
    "sim",
    "simeq",
    "cong",
    "propto",
    "ll",
    "gg",
    "prec",
    "succ",
    "preceq",
    "succeq",
    "subset",
    "supset",
    "subseteq",
    "supseteq",
    "subsetneq",
    "supsetneq",
    "in",
    "notin",
    "ni",
    "mid",
    "nmid",
    "parallel",
    "perp",
    "vdash",
    "dashv",
    "models",
    "coloneqq",
    "coloneq",
    "doteq",
    "triangleq",
    "to",
    "rightarrow",
    "gets",
    "leftarrow",
    "leftrightarrow",
    "Rightarrow",
    "Leftarrow",
    "Leftrightarrow",
    "implies",
    "Longrightarrow",
    "impliedby",
    "Longleftarrow",
    "iff",
    "Longleftrightarrow",
    "longrightarrow",
    "longleftarrow",
    "longleftrightarrow",
    "mapsto",
    "longmapsto",
    "hookrightarrow",
    "hookleftarrow",
    "uparrow",
    "downarrow",
    "updownarrow",
    "Uparrow",
    "Downarrow",
    "nearrow",
    "searrow",
    "rightleftharpoons",
    "sum",
    "prod",
    "coprod",
    "int",
    "iint",
    "iiint",
    "oint",
    "bigcup",
    "bigcap",
    "bigoplus",
    "bigotimes",
    "bigvee",
    "bigwedge",
    "bigsqcup",
    "langle",
    "rangle",
    "lceil",
    "rceil",
    "lfloor",
    "rfloor",
    "lvert",
    "rvert",
    "lVert",
    "rVert",
    "vert",
    "Vert",
    "|",
    "{",
    "lbrace",
    "}",
    "rbrace",
    "lbrack",
    "rbrack",
    "backslash",
    "infty",
    "partial",
    "nabla",
    "forall",
    "exists",
    "nexists",
    "emptyset",
    "varnothing",
    "neg",
    "lnot",
    "ldots",
    "dots",
    "dotsc",
    "dotsb",
    "cdots",
    "vdots",
    "ddots",
    "prime",
    "angle",
    "measuredangle",
    "triangle",
    "square",
    "Box",
    "blacksquare",
    "qed",
    "Diamond",
    "hbar",
    "hslash",
    "ell",
    "wp",
    "Re",
    "Im",
    "aleph",
    "beth",
    "top",
    "bot",
    "therefore",
    "because",
    "checkmark",
    "degree",
    "flat",
    "sharp",
    "natural",
    "clubsuit",
    "diamondsuit",
    "heartsuit",
    "spadesuit",
    "imath",
    "jmath",
    "S",
    "P",
    "%",
    "$",
    "&",
    "#",
    "_",
];

const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh",
    "coth", "log", "ln", "lg", "exp", "lim", "liminf", "limsup", "max", "min", "sup", "inf", "det",
    "dim", "ker", "deg", "gcd", "lcm", "arg", "hom", "Pr", "mod", "bmod", "tr", "Tr", "rank",
    "sgn", "argmax", "argmin",
];

const ACCENTS: &[&str] = &[
    "hat",
    "widehat",
    "tilde",
    "widetilde",
    "bar",
    "overline",
    "widebar",
    "underline",
    "vec",
    "overrightarrow",
    "overleftarrow",
    "dot",
    "ddot",
    "dddot",
    "acute",
    "grave",
    "check",
    "breve",
    "mathring",
];

/// `input ⇒ output` with italics marked.
fn line(tex: &str) -> String {
    let m = to_unicode_and_check(tex);
    let rt = RichText {
        spans: m
            .italic
            .iter()
            .map(|r| Span {
                range: r.clone(),
                style: ITALIC,
                link: None,
            })
            .collect(),
        text: m.text,
        links: Vec::new(),
    };
    format!("{} ⇒ {}", dump::escape(tex), dump::styled(&rt))
}

fn table(inputs: impl IntoIterator<Item = String>) -> String {
    inputs.into_iter().map(|t| line(&t) + "\n").collect()
}

#[test]
fn symbols() {
    insta::assert_snapshot!(table(SYMBOLS.iter().flat_map(|s| {
        // Alone, and between two variables (spacing depends on the class).
        [format!("\\{s}"), format!("a \\{s} b")]
    })));
}

#[test]
fn functions_and_accents() {
    insta::assert_snapshot!(table(
        FUNCTIONS
            .iter()
            .flat_map(|f| [
                format!("\\{f} x"),
                format!("\\{f}(x)"),
                format!("\\{f}^2 x")
            ])
            .chain(ACCENTS.iter().flat_map(|a| [
                format!("\\{a}{{x}}"),
                format!("\\{a}{{AB}}"),
                format!("\\{a} y")
            ]))
    ));
}

/// Structures, alphabets, spacing, environments and odd or malformed input.
#[test]
fn commands() {
    let cases: &[&str] = &[
        // Every character with a Unicode superscript/subscript form, and some without.
        r"x^{0123456789+-=()} x_{0123456789+-=()} x^{-} x_{\pm}",
        r"x^{abcdefghijklmnoprstuvwxyz} x^{q} x^{ABDEGHIJKLMNOPRTUVW} x^{C}",
        r"x^{\alpha\beta\gamma\delta\theta\varphi\chi} x^{*} x^{\ast} x^{\prime\prime} x^{\circ} x^{\epsilon}",
        r"x_{aehijklmnoprstuvx} x_{b} x_{\beta\gamma\rho\varphi\chi} x_{\alpha}",
        // Nested groups inherit script spacing and upright font context.
        r"x^{{a+b}} x_{{a=b}} \mathrm{{x}+{y}}",
        // Raw text keeps escaped punctuation and drops unsupported commands.
        r"\text{a\@b\unknown c}",
        // One unmatched parenthesis does not make an expression fully wrapped.
        r"\frac{(a+b}{c} \frac{a+b)}{c}",
        r"x^{''}",
        // Alphabet fallbacks: digits and non-letters keep their form.
        r"\mathcal{A1!} \mathscr{z9} \mathfrak{x7?} \mathbb{0 9}",
        r"\alpha\beta\gamma\delta\epsilon\varepsilon\zeta\eta\theta\vartheta\iota\kappa\lambda\mu\nu\xi\omicron\pi\varpi\rho\varrho\sigma\varsigma\tau\upsilon\phi\varphi\chi\psi\omega",
        r"\Gamma\Delta\Theta\Lambda\Xi\Pi\Sigma\Upsilon\Phi\Psi\Omega",
        r"a \pm b \mp c \times d \div e \cdot f \ast g \star h \circ i \bullet j",
        r"a \oplus b \ominus c \otimes d \odot e \cup f \cap g \sqcup h \sqcap i \wedge j \land k \vee l \lor m \setminus n \dagger o \ddagger p \amalg q",
        r"a \le b \leq c \ge d \geq e \leqslant f \geqslant g \ne h \neq i \approx j \equiv k \sim l \simeq m \cong n \propto o",
        r"a \ll b \gg c \prec d \succ e \preceq f \succeq g \subset h \supset i \subseteq j \supseteq k \subsetneq l \supsetneq m",
        r"a \in A \notin B \ni c \mid d \nmid e \parallel f \perp g \vdash h \dashv i \models j \coloneqq k \doteq l \triangleq m",
        r"\to \rightarrow \gets \leftarrow \leftrightarrow \Rightarrow \Leftarrow \Leftrightarrow \implies \impliedby \iff",
        r"\longrightarrow \longleftarrow \longleftrightarrow \mapsto \longmapsto \hookrightarrow \hookleftarrow \uparrow \downarrow \updownarrow \Uparrow \Downarrow \nearrow \searrow \rightleftharpoons",
        r"\sum \prod \coprod \int \iint \iiint \oint \bigcup \bigcap \bigoplus \bigotimes \bigvee \bigwedge \bigsqcup",
        r"\langle x \rangle \lceil x \rceil \lfloor x \rfloor \lvert x \rvert \lVert x \rVert \vert \Vert \| \{ x \} \lbrace \rbrace \lbrack \rbrack \backslash",
        r"\infty \partial \nabla \forall \exists \nexists \emptyset \varnothing \neg \lnot \ldots \dots \cdots \vdots \ddots \prime \angle",
        r"\triangle \square \Box \blacksquare \qed \Diamond \hbar \hslash \ell \wp \Re \Im \aleph \beth \top \bot \therefore \because",
        r"\checkmark \degree \flat \sharp \natural \clubsuit \diamondsuit \heartsuit \spadesuit \imath \jmath \S \P \% \$ \& \# \_",
        r"\frac{1}{2} \frac{1}{3} \frac{2}{3} \frac14 \frac{3}{4} \frac{1}{5} \frac{1}{6} \frac{1}{8} \frac{1}{7}",
        r"\frac{a}{b} \dfrac{x+1}{y} \tfrac{a}{b c} \cfrac{(a+b)}{2} \frac{-x}{2} \frac{a/b}{c}",
        r"\binom{n}{k} \dbinom{n+1}{2} \tbinom ab",
        r"\sqrt{2} \sqrt[3]{x} \sqrt[4]{x} \sqrt[n]{x} \sqrt[n+1]{x} \sqrt{x+1} \sqrt[q]{y}",
        r"x^2 x_i x^{10} x_{ij} x^{-1} x^{n+1} x_{a,b} x^{\alpha} x_\beta x^{(n)} x^{\prime} x^{*} x^\circ x'' f'''",
        r"x^{qx} x_{y} x_{qz} e^{i\pi} a^{b^c} x_{1}^{2} x^ x_ ^2 _3",
        r"\hat{x} \widehat{xy} \tilde{a} \widetilde{ab} \bar{x} \overline{AB} \widebar{z} \underline{u} \vec{v} \overrightarrow{AB} \overleftarrow{CD}",
        r"\dot{x} \ddot{y} \dddot{z} \acute{e} \grave{a} \check{c} \breve{u} \mathring{A}",
        r"\mathbb{RNZQCHP} \mathbb{ab12} \Bbb{R} \mathcal{ABEFHILMRZ} \mathcal{ego} \mathscr{L} \mathfrak{CHIRZag} \mathbb{x!}",
        r"\mathrm{d}x \mathbf{v} \mathsf{S} \mathtt{tt} \mathit{if} \mathnormal{n} \boldsymbol{\beta} \bm{x} \operatorname{sgn} x \operatorname*{argmax}_x \mathop{lim} y \mathup{e}",
        r"\text{if } x > 0 \textrm{rm} \textnormal{n} \textup{u} \mbox{m} \hbox{h} \textsf{sf} \texttt{tt} \textmd{md} \textit{it} \emph{em} \textsl{sl} \textbf{bf}",
        r"\text{a {nested} \alpha b_c^d & e} \text x",
        r"\sin x \cos(x) \tan^2 x \log_2 n \ln x \exp x \lim_{x \to 0} \max \min \sup \inf \det A \gcd(a,b) \Pr(A) \arg z \ker f \dim V \deg p \hom \lcm \mod \bmod \tr \Tr \rank \sgn \argmax \argmin \lg \sec \csc \cot \arcsin \arccos \arctan \sinh \cosh \tanh \coth \liminf \limsup",
        r"\sin \left( x \right) \sin\limits",
        r"a\,b\:c\>d\;e\quad f\qquad g\ h\!i~j \thinspace k \medspace l \thickspace m \negthinspace n",
        r"a \\ b \newline c \cr d",
        r"\left( \frac{a}{b} \right) \left. x \right| \middle| \big( \Big[ \bigg\{ \Bigg| \bigl( \bigr) \Bigl( \Bigr) \biggl \biggr \Biggl \Biggr",
        r"\displaystyle x \textstyle y \scriptstyle z \scriptscriptstyle w \limits \nolimits \nonumber \notag \mathstrut \strut \relax \allowbreak",
        r"\label{eq:1} x \hspace{1em} y \vspace{2pt} z \kern{1pt} w \mkern{2mu} v \hskip{3pt} u \phantom{xyz} t",
        r"\mathbin{\star} \mathrel{\sim} a \mathbin{+} b \mathrel{=} c \mathord{x} \mathopen{(} \mathclose{)} \mathpunct{,} \mathinner{y}",
        r"\underbrace{a+b}_{n} \overbrace{c}^{m} \boxed{x} \fbox{y} \smash{z} \cancel{a} \bcancel{b} \xcancel{c} \sout{d}",
        r"\color{red} x \textcolor{blue}{y} \colorbox{yellow}{z}",
        r"\overset{!}{=} \stackrel{def}{=} \underset{x}{\arg\min}",
        r"a \not= b \not\in C \not\subset D \not\subseteq E \not\equiv f \not\sim g \not< h \not\le i",
        r"x \tag{1} y \tag*{A}",
        r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
        r"\begin{bmatrix}1&2\end{bmatrix} \begin{Bmatrix}x\end{Bmatrix} \begin{vmatrix}a\end{vmatrix} \begin{Vmatrix}b\end{Vmatrix} \begin{matrix}m\end{matrix}",
        r"f(x) = \begin{cases} 1 & x > 0 \\ 0 & \text{otherwise} \end{cases}",
        r"\begin{dcases} a \end{dcases} \begin{aligned} x &= 1 \\ y &= 2 \end{aligned} \begin{align*} z \end{align*}",
        r"\begin{array}{cc} a & b \end{array} \begin{tabular}{l} t \end{tabular} \begin{unknown} u \end{unknown}",
        r"\begin{pmatrix} a \begin{bmatrix} b \end{bmatrix} \end{pmatrix}",
        r"\foo \bar \baz{x} \unknown[opt]{arg} \@ \1",
        r"a % comment
 b",
        r"{a}{b}{{c}}",
        r"-x + -y = -(a - b) \cdot -c",
        r"a = b, c; d : e < f > g",
        r"(a) [b] \{c\}",
        r"f(x, y)_{i}",
        r"\operatorname{Var}(X) = E[X^2] - E[X]^2",
        r"\int_0^1 f(x)\,dx \oint_C \sum_{n=0}^{\infty} \frac{1}{n!}",
        r"{",
        r"}",
        r"{{{",
        r"}}}",
        r"\frac{a",
        r"\frac",
        r"\sqrt[",
        r"\sqrt[3",
        r"\begin{pmatrix} a",
        r"\begin",
        r"\end{pmatrix}",
        r"x^",
        r"x_",
        r"x^{",
        r"\",
        r"a\",
        r"\\",
        r"\text{",
        r"\mathbb{",
        r"\left",
        r"\not",
        r"\hat",
        r"^",
        r"_",
        r"&",
        r"%",
        r"''",
        r"",
        r"   ",
        r"\,",
        r"\quad x \quad",
        r"日本 é \alpha",
        r"x^é x_日",
        r"\mathbb{é}",
        r"\text{日本語}",
        r"\frac{é}{日}",
        r"\hat{é}",
        r"\sqrt[é]{x}",
        r"\mathit{\alpha \mathrm{b} c}",
        r"\mathrm{\mathit{x}}",
        r"\boldsymbol{}",
        r"\mathit{}",
        r"\textit{ a }",
        r"x\textit{ }",
        r" \textit{ a b } ",
        r"~x~",
        r"\mathbb{R}^{n \times m}",
        r"x^{\frac{1}{2}}",
        r"x_{\text{max}}",
        r"\vec{\alpha}",
        r"\hat{}",
        r"\sqrt{}",
        r"\frac{}{}",
        r"\binom{}{}",
        r"x^{}",
        r"\left\{ x \right\}",
        r"\left\langle x \right\rangle",
        r"a\mid b",
        r"\{ x \mid x > 0 \}",
    ];
    insta::assert_snapshot!(table(cases.iter().map(|s| s.to_string())));
}

#[test]
fn symbol_list_matches_the_source() {
    let src = include_str!("../src/latex.rs");
    let section = |from: &str, to: &str| {
        let start = src
            .find(from)
            .unwrap_or_else(|| panic!("{from:?} not found"));
        let end = start + src[start..].find(to).unwrap();
        &src[start..end]
    };
    let literals = |part: &str| {
        let mut out = BTreeSet::new();
        for line in part.lines() {
            // Only the names on the left of `=>`.
            let Some((names, _)) = line.split_once("=>") else {
                continue;
            };
            let mut rest = names;
            while let Some(i) = rest.find('"') {
                let end = rest[i + 1..].find('"').unwrap();
                out.insert(rest[i + 1..i + 1 + end].replace("\\\\", "\\"));
                rest = &rest[i + end + 2..];
            }
        }
        out
    };
    let set = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
    assert_eq!(
        set(SYMBOLS),
        literals(section("fn symbol(name", "_ => return None"))
    );
    assert_eq!(
        set(ACCENTS),
        literals(section("fn accent(name", "_ => return None"))
    );
    // `is_function` is one `matches!` with `|`-separated names.
    let f = section("fn is_function", "\n}\n");
    let mut funcs = BTreeSet::new();
    let mut rest = f;
    while let Some(i) = rest.find('"') {
        let end = rest[i + 1..].find('"').unwrap();
        funcs.insert(rest[i + 1..i + 1 + end].to_string());
        rest = &rest[i + end + 2..];
    }
    assert_eq!(set(FUNCTIONS), funcs);
}
