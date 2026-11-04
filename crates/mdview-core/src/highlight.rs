//! Small, dependency-free syntax highlighter for fenced code blocks.
//!
//! It's a set of hand-written lexers (one generic, configured per language,
//! plus a few line-based ones), not a grammar engine. That keeps it fast
//! enough to run inside `markdown::parse` (microseconds per block), so code is
//! already colored in the first frame and nothing is loaded at startup.
//! Like `markdown.rs`, this module must not depend on GPUI.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Keyword,
    Type,
    Function,
    String,
    Number,
    Comment,
    /// `true`, `null`, `self`, `ALL_CAPS`, HTML entities.
    Constant,
    /// Attributes, decorators, macros, preprocessor lines, lifetimes.
    Attr,
    /// Shell/PHP/Perl `$variables`, YAML anchors.
    Variable,
    /// Markup tag names.
    Tag,
    /// Object keys, markup attributes, CSS properties.
    Property,
    /// Diff: added line.
    Inserted,
    /// Diff: removed line.
    Deleted,
    /// Diff headers and hunk markers.
    Meta,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Highlight {
    pub range: Range<usize>,
    pub tok: Tok,
}

/// Returns sorted, non-overlapping highlights. Unhighlighted text is plain.
pub fn highlight(lang: &str, code: &str) -> Vec<Highlight> {
    let lang = lang.to_ascii_lowercase();
    let mut out = Out(Vec::new());
    match lang.as_str() {
        "diff" | "patch" => diff(code, &mut out),
        "sh" | "bash" | "shell" | "zsh" | "fish" | "ksh" | "console" | "shellsession"
        | "terminal" => shell(code, &mut out),
        "html" | "xml" | "svg" | "xhtml" | "vue" | "plist" => markup(code, &mut out),
        "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "properties" | "editorconfig"
        | "gitconfig" | "desktop" => config(code, &mut out),
        _ => {
            if let Some(def) = lang_def(&lang) {
                generic(code, def, &mut out)
            }
        }
    }
    out.0
}

struct Out(Vec<Highlight>);

impl Out {
    fn push(&mut self, range: Range<usize>, tok: Tok) {
        if range.is_empty() {
            return;
        }
        if let Some(last) = self.0.last_mut()
            && last.tok == tok
            && last.range.end == range.start
        {
            last.range.end = range.end;
            return;
        }
        self.0.push(Highlight { range, tok });
    }
}

/// Per-language settings for the generic lexer. Word lists are
/// space-separated.
struct Lang {
    keywords: &'static str,
    types: &'static str,
    constants: &'static str,
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// Quote characters that start a string.
    quotes: &'static [u8],
    /// `"""` / `'''` strings (Python, Kotlin, Swift...).
    triple_quotes: bool,
    /// Identifiers starting with an uppercase letter are types.
    caps_are_types: bool,
    /// Keywords match case-insensitively (SQL).
    ignore_case: bool,
    /// `@name` is an attribute/decorator (or a keyword in CSS).
    at_attrs: bool,
    /// `$name` is a variable.
    dollar_vars: bool,
    /// Rust-specific bits: `name!` macros, `#[attr]`, lifetimes, raw strings.
    rust: bool,
    /// C preprocessor lines (`#include`).
    preprocessor: bool,
    /// `"key":` strings are properties (JSON).
    json_keys: bool,
    /// `name:` inside braces is a property (CSS).
    css: bool,
    /// Words that introduce a function name (`fn`, `def`, `function`).
    fn_keywords: &'static str,
}

const BASE: Lang = Lang {
    keywords: "",
    types: "",
    constants: "true false null",
    line_comments: &[],
    block_comment: None,
    quotes: b"\"'",
    triple_quotes: false,
    caps_are_types: false,
    ignore_case: false,
    at_attrs: false,
    dollar_vars: false,
    rust: false,
    preprocessor: false,
    json_keys: false,
    css: false,
    fn_keywords: "",
};

const C_TYPES: &str = "void char short int long float double signed unsigned bool size_t ssize_t \
    int8_t int16_t int32_t int64_t uint8_t uint16_t uint32_t uint64_t uintptr_t intptr_t \
    ptrdiff_t wchar_t FILE auto";
const C_KEYWORDS: &str = "if else for while do switch case default break continue return goto \
    struct union enum typedef sizeof static extern const volatile register inline restrict \
    _Bool _Static_assert";
const CPP_KEYWORDS: &str = "if else for while do switch case default break continue return goto \
    struct union enum typedef sizeof static extern const volatile register inline class \
    namespace template typename public private protected virtual override final friend \
    operator new delete this using try catch throw noexcept constexpr consteval constinit \
    explicit mutable static_cast dynamic_cast const_cast reinterpret_cast decltype concept \
    requires co_await co_return co_yield export import module";
const JS_KEYWORDS: &str = "async await break case catch class const continue debugger default \
    delete do else export extends finally for from function get if import in instanceof let \
    new of return set static super switch throw try typeof var void while with yield as";
const TS_EXTRA: &str = "async await break case catch class const continue debugger default \
    delete do else export extends finally for from function get if import in instanceof let \
    new of return set static super switch throw try typeof var void while with yield as \
    interface type enum implements declare namespace module abstract public private \
    protected readonly keyof infer is satisfies";

fn lang_def(lang: &str) -> Option<&'static Lang> {
    static RUST: Lang = Lang {
        keywords: "as async await break const continue crate dyn else enum extern fn for if impl \
            in let loop match mod move mut pub ref return static struct trait type unsafe use \
            where while union macro_rules yield",
        types: "i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize f32 f64 bool char str",
        constants: "true false self Self super None Some Ok Err",
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: b"\"",
        caps_are_types: true,
        rust: true,
        fn_keywords: "fn",
        ..BASE
    };
    static C: Lang = Lang {
        keywords: C_KEYWORDS,
        types: C_TYPES,
        constants: "true false NULL nullptr",
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        preprocessor: true,
        ..BASE
    };
    static CPP: Lang = Lang {
        keywords: CPP_KEYWORDS,
        types: C_TYPES,
        constants: "true false NULL nullptr",
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        preprocessor: true,
        caps_are_types: true,
        ..BASE
    };
    static CSHARP: Lang = Lang {
        keywords: "abstract as async await base break case catch checked class const continue \
            default delegate do else enum event explicit extern finally fixed for foreach goto \
            if implicit in interface internal is lock namespace new operator out override \
            params private protected public readonly ref return sealed sizeof stackalloc \
            static struct switch this throw try typeof unchecked unsafe using var virtual \
            volatile while yield record init get set when where",
        types: "bool byte char decimal double float int long object sbyte short string uint \
            ulong ushort void dynamic",
        constants: "true false null this",
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        caps_are_types: true,
        preprocessor: true,
        ..BASE
    };
    static GO: Lang = Lang {
        keywords: "break case chan const continue default defer else fallthrough for func go \
            goto if import interface map package range return select struct switch type var",
        types: "bool byte complex64 complex128 error float32 float64 int int8 int16 int32 int64 \
            rune string uint uint8 uint16 uint32 uint64 uintptr any",
        constants: "true false nil iota",
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: b"\"'`",
        fn_keywords: "func",
        ..BASE
    };
    static JAVA: Lang = Lang {
        keywords: "abstract assert break case catch class const continue default do else enum \
            extends final finally for goto if implements import instanceof interface native new \
            package private protected public return static strictfp super switch synchronized \
            throw throws transient try volatile while var record sealed permits yield",
        types: "boolean byte char double float int long short void",
        constants: "true false null this",
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        caps_are_types: true,
        at_attrs: true,
        ..BASE
    };
    static KOTLIN: Lang = Lang {
        keywords: "as break class continue do else for fun if in interface is object package \
            return super throw try typealias typeof val var when while by catch constructor \
            companion data enum finally get import init inline internal lateinit open \
            operator out override private protected public sealed set suspend vararg where",
        constants: "true false null this it",
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        triple_quotes: true,
        caps_are_types: true,
        at_attrs: true,
        fn_keywords: "fun",
        ..BASE
    };
    static SWIFT: Lang = Lang {
        keywords: "associatedtype class deinit enum extension fileprivate func import init \
            inout internal let open operator private protocol public rethrows static struct \
            subscript typealias var break case continue default defer do else fallthrough for \
            guard if in repeat return switch where while as catch is throw throws try async \
            await some any mutating",
        constants: "true false nil self Self super",
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: b"\"",
        triple_quotes: true,
        caps_are_types: true,
        at_attrs: true,
        fn_keywords: "func",
        ..BASE
    };
    static DART: Lang = Lang {
        keywords: "abstract as assert async await break case catch class const continue \
            covariant default deferred do dynamic else enum export extends extension external \
            factory final finally for get if implements import in interface is late library \
            mixin new operator part required rethrow return set static super switch sync this \
            throw try typedef var void while with yield",
        constants: "true false null this",
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        triple_quotes: true,
        caps_are_types: true,
        at_attrs: true,
        ..BASE
    };
    static SCALA: Lang = Lang {
        keywords: "abstract case catch class def do else extends final finally for forSome if \
            implicit import lazy match new object override package private protected return \
            sealed super throw trait try type val var while with yield given using enum then",
        constants: "true false null this",
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        triple_quotes: true,
        caps_are_types: true,
        at_attrs: true,
        fn_keywords: "def",
        ..BASE
    };
    static JS: Lang = Lang {
        keywords: JS_KEYWORDS,
        constants: "true false null undefined this NaN Infinity",
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: b"\"'`",
        caps_are_types: true,
        fn_keywords: "function",
        ..BASE
    };
    static TS: Lang = Lang {
        keywords: TS_EXTRA,
        types: "string number boolean any unknown never void object bigint symbol",
        constants: "true false null undefined this NaN Infinity",
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: b"\"'`",
        caps_are_types: true,
        at_attrs: true,
        fn_keywords: "function",
        ..BASE
    };
    static PYTHON: Lang = Lang {
        keywords: "and as assert async await break class continue def del elif else except \
            finally for from global if import in is lambda nonlocal not or pass raise return \
            try while with yield match case print",
        types: "int float str bool list dict set tuple bytes object type complex frozenset",
        constants: "True False None self cls",
        line_comments: &["#"],
        triple_quotes: true,
        caps_are_types: true,
        at_attrs: true,
        fn_keywords: "def class",
        ..BASE
    };
    static RUBY: Lang = Lang {
        keywords: "alias and begin break case class def defined? do else elsif end ensure for \
            if in module next not or redo rescue retry return super then undef unless until \
            when while yield require require_relative include extend attr_reader attr_writer \
            attr_accessor private protected public",
        constants: "true false nil self",
        line_comments: &["#"],
        quotes: b"\"'`",
        caps_are_types: true,
        fn_keywords: "def",
        ..BASE
    };
    static PHP: Lang = Lang {
        keywords: "abstract and array as break callable case catch class clone const continue \
            declare default do echo else elseif empty enddeclare endfor endforeach endif \
            endswitch endwhile extends final finally fn for foreach function global goto if \
            implements include include_once instanceof insteadof interface isset list match \
            namespace new or print private protected public readonly require require_once \
            return static switch throw trait try unset use var while xor yield",
        constants: "true false null TRUE FALSE NULL",
        line_comments: &["//", "#"],
        block_comment: Some(("/*", "*/")),
        caps_are_types: true,
        dollar_vars: true,
        fn_keywords: "function",
        ..BASE
    };
    static LUA: Lang = Lang {
        keywords: "and break do else elseif end for function goto if in local not or repeat \
            return then until while",
        constants: "true false nil self",
        line_comments: &["--"],
        block_comment: Some(("--[[", "]]")),
        fn_keywords: "function",
        ..BASE
    };
    static SQL: Lang = Lang {
        keywords: "select from where and or not insert into values update set delete create \
            table drop alter add column index view join inner left right outer full on as \
            group by order having limit offset union all distinct case when then else end \
            is in exists between like primary key foreign references default unique check \
            constraint begin commit rollback transaction with recursive returning asc desc \
            cascade if replace grant revoke database schema trigger procedure function return \
            returns declare cross natural using over partition window",
        types: "int integer smallint bigint serial bigserial real double precision float \
            numeric decimal varchar char text boolean bool date time timestamp timestamptz \
            interval uuid json jsonb blob bytea",
        constants: "true false null",
        line_comments: &["--"],
        block_comment: Some(("/*", "*/")),
        ignore_case: true,
        ..BASE
    };
    static CSS: Lang = Lang {
        keywords: "important",
        constants: "",
        block_comment: Some(("/*", "*/")),
        at_attrs: true,
        css: true,
        ..BASE
    };
    static JSON: Lang = Lang {
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: b"\"",
        json_keys: true,
        ..BASE
    };
    static HASKELL: Lang = Lang {
        keywords: "case class data default deriving do else foreign if import in infix infixl \
            infixr instance let module newtype of then type where qualified as hiding forall",
        constants: "True False Nothing Just",
        line_comments: &["--"],
        block_comment: Some(("{-", "-}")),
        quotes: b"\"",
        caps_are_types: true,
        ..BASE
    };
    static ZIG: Lang = Lang {
        keywords: "addrspace align allowzero and anyframe anytype asm async await break \
            callconv catch comptime const continue defer else enum errdefer error export extern \
            fn for if inline noalias nosuspend noinline opaque or orelse packed pub resume \
            return linksection struct suspend switch test threadlocal try union unreachable \
            usingnamespace var volatile while",
        types: "i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize f16 f32 f64 f128 bool \
            void noreturn type anyerror comptime_int comptime_float",
        constants: "true false null undefined",
        line_comments: &["//"],
        caps_are_types: true,
        fn_keywords: "fn",
        ..BASE
    };
    static NIX: Lang = Lang {
        keywords: "let in with rec inherit if then else assert import",
        constants: "true false null",
        line_comments: &["#"],
        block_comment: Some(("/*", "*/")),
        quotes: b"\"",
        ..BASE
    };
    static ELIXIR: Lang = Lang {
        keywords: "def defp defmodule defmacro defstruct defprotocol defimpl do end fn if \
            else unless case cond with for receive try catch rescue after raise import alias \
            require use when and or not in quote unquote",
        constants: "true false nil",
        line_comments: &["#"],
        triple_quotes: true,
        caps_are_types: true,
        fn_keywords: "def defp",
        ..BASE
    };
    static MAKE: Lang = Lang {
        keywords: "ifeq ifneq ifdef ifndef else endif include define endef export override",
        constants: "",
        line_comments: &["#"],
        dollar_vars: true,
        ..BASE
    };
    static DOCKER: Lang = Lang {
        keywords: "FROM RUN CMD LABEL EXPOSE ENV ADD COPY ENTRYPOINT VOLUME USER WORKDIR ARG \
            ONBUILD STOPSIGNAL HEALTHCHECK SHELL AS",
        constants: "",
        line_comments: &["#"],
        dollar_vars: true,
        ..BASE
    };

    Some(match lang {
        "rust" | "rs" => &RUST,
        "c" | "h" => &C,
        "cpp" | "c++" | "cc" | "cxx" | "hpp" | "hxx" | "objc" | "objective-c" | "cuda" | "glsl"
        | "hlsl" | "wgsl" | "metal" => &CPP,
        "cs" | "csharp" | "c#" => &CSHARP,
        "go" | "golang" => &GO,
        "java" | "groovy" => &JAVA,
        "kotlin" | "kt" | "kts" => &KOTLIN,
        "swift" => &SWIFT,
        "dart" => &DART,
        "scala" => &SCALA,
        "js" | "javascript" | "jsx" | "mjs" | "cjs" => &JS,
        "ts" | "typescript" | "tsx" | "mts" => &TS,
        "py" | "python" | "python3" | "py3" | "pyi" | "gdscript" => &PYTHON,
        "rb" | "ruby" => &RUBY,
        "php" => &PHP,
        "lua" => &LUA,
        "sql" | "psql" | "mysql" | "sqlite" | "postgresql" | "plsql" => &SQL,
        "css" | "scss" | "sass" | "less" => &CSS,
        "json" | "jsonc" | "json5" | "geojson" => &JSON,
        "haskell" | "hs" => &HASKELL,
        "zig" => &ZIG,
        "nix" => &NIX,
        "elixir" | "ex" | "exs" => &ELIXIR,
        "make" | "makefile" | "mk" => &MAKE,
        "dockerfile" | "docker" | "containerfile" => &DOCKER,
        _ => return None,
    })
}

fn has_word(list: &str, w: &str, ignore_case: bool) -> bool {
    list.split_ascii_whitespace().any(|k| {
        if ignore_case {
            k.eq_ignore_ascii_case(w)
        } else {
            k == w
        }
    })
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// End of a number starting at `i`: digits, letters (hex, suffixes,
/// exponents), `_`, and a `.` only when a digit follows (so `0..10` stays
/// a range).
fn number_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() {
        let c = b[i];
        let more = c.is_ascii_alphanumeric()
            || c == b'_'
            || (c == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
            || ((c == b'+' || c == b'-') && matches!(b[i - 1], b'e' | b'E') && !is_hex(b, i));
        if !more {
            break;
        }
        i += 1;
    }
    i
}

fn is_hex(b: &[u8], end: usize) -> bool {
    let mut s = end;
    while s > 0 && b[s - 1].is_ascii_alphanumeric() {
        s -= 1;
    }
    b[s..end].starts_with(b"0x") || b[s..end].starts_with(b"0X")
}

/// End of a quoted string starting at `i` (the opening quote). Strings stop
/// at the end of the line unless they are backtick strings.
fn string_end(b: &[u8], i: usize, q: u8, escapes: bool) -> usize {
    let mut j = i + 1;
    while j < b.len() {
        let c = b[j];
        if escapes && c == b'\\' {
            j += 2;
            continue;
        }
        if c == q {
            return j + 1;
        }
        if c == b'\n' && q != b'`' {
            return j;
        }
        j += 1;
    }
    b.len()
}

fn find(b: &[u8], from: usize, pat: &str) -> Option<usize> {
    let p = pat.as_bytes();
    if p.is_empty() || from > b.len() {
        return None;
    }
    b[from..]
        .windows(p.len())
        .position(|w| w == p)
        .map(|x| x + from)
}

fn line_end(b: &[u8], i: usize) -> usize {
    b[i..]
        .iter()
        .position(|&c| c == b'\n')
        .map_or(b.len(), |p| p + i)
}

fn next_non_space(b: &[u8], mut i: usize) -> Option<u8> {
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    b.get(i).copied()
}

fn generic(code: &str, l: &Lang, out: &mut Out) {
    let b = code.as_bytes();
    let mut i = 0;
    let mut line_start = true;
    let mut prev_word = "";
    let mut depth = 0i32; // brace depth, for CSS properties
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
            continue;
        }
        let at_line_start = std::mem::replace(&mut line_start, false);
        let prev_space = i == 0 || b[i - 1].is_ascii_whitespace();

        // Comments. Block comments first: Lua's `--[[` starts like `--`.
        if let Some((open, close)) = l.block_comment
            && b[i..].starts_with(open.as_bytes())
        {
            let end = find(b, i + open.len(), close).map_or(b.len(), |e| e + close.len());
            out.push(i..end, Tok::Comment);
            i = end;
            continue;
        }
        if let Some(lc) = l
            .line_comments
            .iter()
            .find(|p| b[i..].starts_with(p.as_bytes()))
            && (*lc != "#" || prev_space)
        {
            let end = line_end(b, i);
            out.push(i..end, Tok::Comment);
            i = end;
            continue;
        }
        if l.preprocessor && c == b'#' && at_line_start {
            let mut j = i + 1;
            while j < b.len() && (b[j] == b' ' || is_ident(b[j])) && b[j] != b'\n' {
                if b[j] == b' ' && j > i + 1 && is_ident(b[j - 1]) {
                    break;
                }
                j += 1;
            }
            out.push(i..j, Tok::Attr);
            i = j;
            continue;
        }

        // Rust specifics.
        if l.rust {
            if c == b'#' && matches!(b.get(i + 1), Some(b'[') | Some(b'!')) {
                // `#[...]` / `#![...]`, with nested brackets.
                let mut j = i + 1;
                let mut d = 0;
                while j < b.len() {
                    match b[j] {
                        b'[' => d += 1,
                        b']' => {
                            d -= 1;
                            if d == 0 {
                                j += 1;
                                break;
                            }
                        }
                        b'\n' if d == 0 => break,
                        _ => {}
                    }
                    j += 1;
                }
                out.push(i..j, Tok::Attr);
                i = j;
                continue;
            }
            // Raw strings: r"..", r#".."#, br"..".
            let r = if c == b'b' && b.get(i + 1) == Some(&b'r') {
                Some(i + 2)
            } else if c == b'r' {
                Some(i + 1)
            } else {
                None
            };
            if let Some(mut j) = r
                && (i == 0 || !is_ident(b[i - 1]))
            {
                let mut hashes = 0;
                while b.get(j) == Some(&b'#') {
                    hashes += 1;
                    j += 1;
                }
                if b.get(j) == Some(&b'"') {
                    let close = format!("\"{}", "#".repeat(hashes));
                    let end = find(b, j + 1, &close).map_or(b.len(), |e| e + close.len());
                    out.push(i..end, Tok::String);
                    i = end;
                    continue;
                }
            }
            if c == b'\'' {
                // Char literal or lifetime?
                let is_char = match b.get(i + 1) {
                    Some(b'\\') => true,
                    Some(&n) if n >= 0x80 => true,
                    Some(_) => b.get(i + 2) == Some(&b'\''),
                    None => false,
                };
                if is_char {
                    let end = string_end(b, i, b'\'', true);
                    out.push(i..end, Tok::String);
                    i = end;
                } else {
                    let mut j = i + 1;
                    while j < b.len() && is_ident(b[j]) {
                        j += 1;
                    }
                    out.push(i..j, Tok::Attr);
                    i = j;
                }
                continue;
            }
        }

        // Strings.
        if l.quotes.contains(&c) {
            let triple = [c, c, c];
            let end = if l.triple_quotes && b[i..].starts_with(&triple) {
                let close = std::str::from_utf8(&triple).unwrap();
                find(b, i + 3, close).map_or(b.len(), |e| e + 3)
            } else {
                string_end(b, i, c, true)
            };
            let tok = if l.json_keys && next_non_space(b, end) == Some(b':') {
                Tok::Property
            } else {
                Tok::String
            };
            out.push(i..end, tok);
            i = end;
            continue;
        }

        // Numbers (and CSS hex colors).
        if c.is_ascii_digit()
            || (c == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit()) && prev_space)
            || (l.css && c == b'#' && depth > 0)
        {
            let end = number_end(b, i + 1);
            out.push(i..end, Tok::Number);
            i = end;
            continue;
        }

        // `@decorator`, `$var`.
        if (l.at_attrs && c == b'@') || (l.dollar_vars && c == b'$') {
            let mut j = i + 1;
            if l.dollar_vars && matches!(b.get(j), Some(b'{') | Some(b'(')) {
                let close = if b[j] == b'{' { b'}' } else { b')' };
                while j < b.len() && b[j] != close && b[j] != b'\n' {
                    j += 1;
                }
                j = (j + 1).min(b.len());
            } else {
                while j < b.len() && (is_ident(b[j]) || b[j] == b'.' || b[j] == b'-') {
                    j += 1;
                }
            }
            if j > i + 1 {
                let tok = if c == b'$' {
                    Tok::Variable
                } else if l.css {
                    Tok::Keyword
                } else {
                    Tok::Attr
                };
                out.push(i..j, tok);
                i = j;
                continue;
            }
        }

        // Identifiers.
        if is_ident_start(c)
            || (l.css
                && c == b'-'
                && b.get(i + 1)
                    .is_some_and(|&n| is_ident_start(n) || n == b'-'))
        {
            let mut j = i + 1;
            while j < b.len() && (is_ident(b[j]) || (l.css && b[j] == b'-')) {
                j += 1;
            }
            // Ruby's `defined?`
            let word = &code[i..j];
            let next = b.get(j).copied();
            let tok = if l.rust && next == Some(b'!') && b.get(j + 1) != Some(&b'=') {
                j += 1;
                Some(Tok::Attr)
            } else if l.css {
                if depth > 0 && next_non_space(b, j) == Some(b':') {
                    Some(Tok::Property)
                } else if depth == 0 {
                    Some(Tok::Tag)
                } else if next == Some(b'(') {
                    Some(Tok::Function)
                } else {
                    None
                }
            } else if has_word(l.keywords, word, l.ignore_case) {
                Some(Tok::Keyword)
            } else if has_word(l.constants, word, l.ignore_case) {
                Some(Tok::Constant)
            } else if has_word(l.types, word, l.ignore_case) {
                Some(Tok::Type)
            } else if has_word(l.fn_keywords, prev_word, false) {
                Some(if word.starts_with(|c: char| c.is_ascii_uppercase()) {
                    Tok::Type
                } else {
                    Tok::Function
                })
            } else if next == Some(b'(') || (l.rust && b[j..].starts_with(b"::<")) {
                Some(Tok::Function)
            } else if word.len() > 1
                && word
                    .bytes()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
                && word.bytes().any(|c| c.is_ascii_uppercase())
            {
                Some(Tok::Constant)
            } else if l.caps_are_types && word.starts_with(|c: char| c.is_ascii_uppercase()) {
                Some(Tok::Type)
            } else {
                None
            };
            if let Some(tok) = tok {
                out.push(i..j, tok);
            }
            prev_word = &code[i..j.min(i + word.len())];
            i = j;
            continue;
        }

        match c {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        prev_word = "";
        // Skip one (possibly multi-byte) character.
        i += utf8_len(c);
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0xF0..=0xFF => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}

fn diff(code: &str, out: &mut Out) {
    let mut pos = 0;
    for line in code.split_inclusive('\n') {
        let r = pos..pos + line.trim_end_matches('\n').len();
        pos += line.len();
        let tok = if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("diff ")
            || line.starts_with("index ")
            || line.starts_with("@@")
        {
            Tok::Meta
        } else if line.starts_with('+') || line.starts_with('>') {
            Tok::Inserted
        } else if line.starts_with('-') || line.starts_with('<') {
            Tok::Deleted
        } else {
            continue;
        };
        out.push(r, tok);
    }
}

const SHELL_KEYWORDS: &str = "if then else elif fi for while until do done case esac in \
    function return export local readonly declare unset set source alias break continue \
    exit eval exec trap shift select time";

/// Shell scripts and terminal sessions: comments, strings, `$vars`, keywords,
/// and the command name at the start of each command.
fn shell(code: &str, out: &mut Out) {
    let b = code.as_bytes();
    let mut i = 0;
    let mut cmd_pos = true;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            cmd_pos = true;
            i += 1;
            // A `$ ` or `> ` prompt at the start of a line.
            if b[i..].starts_with(b"$ ") || b[i..].starts_with(b"% ") {
                out.push(i..i + 1, Tok::Comment);
                i += 2;
            }
            continue;
        }
        if i == 0 && (b.starts_with(b"$ ") || b.starts_with(b"% ")) {
            out.push(0..1, Tok::Comment);
            i = 2;
            continue;
        }
        if c == b' ' || c == b'\t' {
            i += 1;
            continue;
        }
        let prev_space = i == 0 || b[i - 1].is_ascii_whitespace();
        if c == b'#' && prev_space {
            let end = line_end(b, i);
            out.push(i..end, Tok::Comment);
            i = end;
            continue;
        }
        if c == b'"' || c == b'\'' {
            let end = string_end(b, i, c, c == b'"');
            out.push(i..end, Tok::String);
            i = end;
            cmd_pos = false;
            continue;
        }
        if c == b'$' {
            let mut j = i + 1;
            match b.get(j) {
                Some(b'{') | Some(b'(') => {
                    let close = if b[j] == b'{' { b'}' } else { b')' };
                    while j < b.len() && b[j] != close && b[j] != b'\n' {
                        j += 1;
                    }
                    j = (j + 1).min(b.len());
                }
                _ => {
                    while j < b.len() && (is_ident(b[j]) || matches!(b[j], b'?' | b'@' | b'#')) {
                        j += 1;
                        if !is_ident(b[j - 1]) {
                            break;
                        }
                    }
                }
            }
            out.push(i..j, Tok::Variable);
            i = j;
            continue;
        }
        if matches!(c, b'|' | b';' | b'&' | b'(' | b'{' | b'`') {
            cmd_pos = true;
            i += 1;
            continue;
        }
        if c == b'\\' {
            i += 2;
            continue;
        }
        // A word.
        let mut j = i;
        while j < b.len()
            && !b[j].is_ascii_whitespace()
            && !matches!(
                b[j],
                b'|' | b';' | b'&' | b'(' | b')' | b'"' | b'\'' | b'`' | b'$'
            )
        {
            j += 1;
        }
        if j == i {
            i += 1;
            continue;
        }
        let word = &code[i..j];
        if let Some(eq) = word.find('=')
            && cmd_pos
            && eq > 0
            && word[..eq].bytes().all(is_ident)
        {
            // `NAME=value cmd`: the command still comes next.
            out.push(i..i + eq, Tok::Variable);
            i = j;
            continue;
        }
        if has_word(SHELL_KEYWORDS, word, false) {
            out.push(i..j, Tok::Keyword);
            cmd_pos = !matches!(word, "in" | "export" | "local" | "readonly" | "declare");
        } else if cmd_pos {
            out.push(i..j, Tok::Function);
            cmd_pos = false;
        } else if word.starts_with('-') {
            out.push(i..j, Tok::Attr);
        } else if word.bytes().all(|c| c.is_ascii_digit()) {
            out.push(i..j, Tok::Number);
        }
        i = j;
    }
}

/// HTML / XML: comments, tags, attributes, attribute values, entities.
fn markup(code: &str, out: &mut Out) {
    let b = code.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if b[i..].starts_with(b"<!--") {
            let end = find(b, i + 4, "-->").map_or(b.len(), |e| e + 3);
            out.push(i..end, Tok::Comment);
            i = end;
            continue;
        }
        if c == b'&' {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'#') {
                j += 1;
            }
            if b.get(j) == Some(&b';') {
                out.push(i..j + 1, Tok::Constant);
                i = j + 1;
                continue;
            }
        }
        if c == b'<'
            && b.get(i + 1)
                .is_some_and(|&n| n.is_ascii_alphabetic() || matches!(n, b'/' | b'!' | b'?'))
        {
            let mut j = i + 1;
            while j < b.len() && matches!(b[j], b'/' | b'!' | b'?') {
                j += 1;
            }
            let name = j;
            while j < b.len() && (is_ident(b[j]) || matches!(b[j], b'-' | b':' | b'.')) {
                j += 1;
            }
            out.push(name..j, Tok::Tag);
            // Attributes until `>`.
            while j < b.len() && b[j] != b'>' {
                let a = b[j];
                if a == b'"' || a == b'\'' {
                    let end = string_end(b, j, a, false);
                    // Attribute values may span lines.
                    let end = if end < b.len() && b[end - 1] != a {
                        find(b, j + 1, if a == b'"' { "\"" } else { "'" })
                            .map_or(b.len(), |e| e + 1)
                    } else {
                        end
                    };
                    out.push(j..end, Tok::String);
                    j = end;
                } else if is_ident_start(a) {
                    let s = j;
                    while j < b.len() && (is_ident(b[j]) || matches!(b[j], b'-' | b':' | b'.')) {
                        j += 1;
                    }
                    out.push(s..j, Tok::Property);
                } else {
                    j += 1;
                }
            }
            i = (j + 1).min(b.len());
            continue;
        }
        i += 1;
    }
}

/// YAML, TOML and INI: comments, `[sections]`, `key:` / `key =`, and
/// scalar values.
fn config(code: &str, out: &mut Out) {
    let mut pos = 0;
    for line in code.split_inclusive('\n') {
        let start = pos;
        pos += line.len();
        let line = line.trim_end_matches(['\n', '\r']);
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        if i == b.len() {
            continue;
        }
        if b[i] == b'#' || b[i] == b';' {
            out.push(start + i..start + b.len(), Tok::Comment);
            continue;
        }
        if b[i] == b'[' {
            // TOML/INI section header.
            let end = line.rfind(']').map_or(b.len(), |e| e + 1);
            out.push(start + i..start + end, Tok::Type);
            value(line, end, start, out);
            continue;
        }
        if line[i..].starts_with("---") || line[i..].starts_with("...") {
            out.push(start + i..start + b.len(), Tok::Meta);
            continue;
        }
        // YAML list item.
        while b[i..].starts_with(b"- ") {
            i += 2;
        }
        // `key:` (YAML) or `key =` (TOML/INI).
        let key_end = {
            let mut j = i;
            let mut q = None;
            loop {
                if j >= b.len() {
                    break None;
                }
                let c = b[j];
                match q {
                    Some(qc) if c == qc => q = None,
                    Some(_) => {}
                    None if c == b'"' || c == b'\'' => q = Some(c),
                    None if c == b'=' => break Some(j),
                    None if c == b':' && (j + 1 == b.len() || b[j + 1] == b' ') => break Some(j),
                    None if c == b' '
                        && j > i
                        && !line[j..].trim_start().starts_with(['=', ':']) =>
                    {
                        break None;
                    }
                    None => {}
                }
                j += 1;
            }
        };
        match key_end {
            Some(k) if k > i => {
                let key = line[i..k].trim_end();
                out.push(start + i..start + i + key.len(), Tok::Property);
                value(line, k + 1, start, out);
            }
            _ => value(line, i, start, out),
        }
    }
}

/// Highlights a scalar value (rest of a config line from `from`).
fn value(line: &str, from: usize, base: usize, out: &mut Out) {
    let b = line.as_bytes();
    let mut i = from;
    while i < b.len() {
        let c = b[i];
        let prev_space = i == 0 || b[i - 1].is_ascii_whitespace();
        if c == b'#' && prev_space {
            out.push(base + i..base + b.len(), Tok::Comment);
            return;
        }
        if c == b'"' || c == b'\'' {
            let end = string_end(b, i, c, c == b'"');
            out.push(base + i..base + end, Tok::String);
            i = end;
            continue;
        }
        if (c == b'&' || c == b'*') && prev_space && b.get(i + 1).is_some_and(|&n| is_ident(n)) {
            let mut j = i + 1;
            while j < b.len() && (is_ident(b[j]) || b[j] == b'-') {
                j += 1;
            }
            out.push(base + i..base + j, Tok::Variable);
            i = j;
            continue;
        }
        if c.is_ascii_alphanumeric() || c == b'-' || c == b'+' || c == b'.' {
            let mut j = i;
            while j < b.len() && !b[j].is_ascii_whitespace() && !matches!(b[j], b',' | b']' | b'}')
            {
                j += 1;
            }
            let word = &line[i..j];
            let is_num = word
                .trim_start_matches(['-', '+'])
                .starts_with(|c: char| c.is_ascii_digit())
                && word.bytes().all(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'+' | b':')
                });
            if is_num {
                out.push(base + i..base + j, Tok::Number);
            } else if matches!(
                word,
                "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "True" | "False" | "~"
            ) {
                out.push(base + i..base + j, Tok::Constant);
            }
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(lang: &str, code: &str) -> Vec<(String, Tok)> {
        highlight(lang, code)
            .into_iter()
            .map(|h| (code[h.range].to_string(), h.tok))
            .collect()
    }

    #[test]
    fn rust() {
        let t = toks(
            "rust",
            "#[derive(Debug)]\nfn main<'a>() { let x: u32 = 0x1F; println!(\"hi {x}\"); // done\n}",
        );
        assert!(t.contains(&("#[derive(Debug)]".into(), Tok::Attr)));
        assert!(t.contains(&("fn".into(), Tok::Keyword)));
        assert!(t.contains(&("main".into(), Tok::Function)));
        assert!(t.contains(&("'a".into(), Tok::Attr)));
        assert!(t.contains(&("u32".into(), Tok::Type)));
        assert!(t.contains(&("0x1F".into(), Tok::Number)));
        assert!(t.contains(&("println!".into(), Tok::Attr)));
        assert!(t.contains(&("\"hi {x}\"".into(), Tok::String)));
        assert!(t.contains(&("// done".into(), Tok::Comment)));
    }

    #[test]
    fn ranges_are_sorted_and_char_boundaries() {
        let code = "let s = \"héllo\"; // ünïcode\nlet r = 0..10;";
        let hs = highlight("rust", code);
        let mut last = 0;
        for h in &hs {
            assert!(h.range.start >= last);
            assert!(code.is_char_boundary(h.range.start) && code.is_char_boundary(h.range.end));
            last = h.range.end;
        }
        assert!(toks("rust", code).contains(&("0".into(), Tok::Number)));
    }

    #[test]
    fn python_and_json_and_shell() {
        let t = toks(
            "python",
            "@dataclass\ndef f(x):\n    return \"\"\"doc\"\"\" # c",
        );
        assert!(t.contains(&("@dataclass".into(), Tok::Attr)));
        assert!(t.contains(&("f".into(), Tok::Function)));
        assert!(t.contains(&("\"\"\"doc\"\"\"".into(), Tok::String)));
        assert!(t.contains(&("# c".into(), Tok::Comment)));

        let t = toks("json", "{\"a\": [1, true, \"x\"]}");
        assert_eq!(
            t,
            [
                ("\"a\"".into(), Tok::Property),
                ("1".into(), Tok::Number),
                ("true".into(), Tok::Constant),
                ("\"x\"".into(), Tok::String)
            ]
        );

        let t = toks(
            "sh",
            "FOO=1 cargo build --release # go\necho \"$HOME\" | grep x",
        );
        assert!(t.contains(&("FOO".into(), Tok::Variable)));
        assert!(t.contains(&("cargo".into(), Tok::Function)));
        assert!(t.contains(&("--release".into(), Tok::Attr)));
        assert!(t.contains(&("# go".into(), Tok::Comment)));
        assert!(t.contains(&("grep".into(), Tok::Function)));
    }

    #[test]
    fn config_and_diff_and_unknown() {
        let t = toks("toml", "[package]\nname = \"x\" # n\nlto = true");
        assert!(t.contains(&("[package]".into(), Tok::Type)));
        assert!(t.contains(&("name".into(), Tok::Property)));
        assert!(t.contains(&("true".into(), Tok::Constant)));
        let t = toks("yaml", "key: 12\n- item: no");
        assert!(t.contains(&("key".into(), Tok::Property)));
        assert!(t.contains(&("12".into(), Tok::Number)));
        assert!(t.contains(&("item".into(), Tok::Property)));
        let t = toks("diff", "@@ -1 +1 @@\n-a\n+b\n c");
        assert_eq!(t[1], ("-a".into(), Tok::Deleted));
        assert_eq!(t[2], ("+b".into(), Tok::Inserted));
        assert!(highlight("brainfuck", "+++").is_empty());
    }
}
