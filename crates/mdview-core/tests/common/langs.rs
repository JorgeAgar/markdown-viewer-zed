//! Every language name `highlight()` knows, grouped by lexer: the first name
//! of each group is the canonical one, the rest are aliases that must behave
//! exactly the same. `tests/highlight.rs` checks this list against the
//! `match` arms in `src/highlight.rs`, so a new language has to be added here
//! (and gets a sample snapshot).

#![allow(dead_code)]

pub const LANG_GROUPS: &[&[&str]] = &[
    // Line-based lexers.
    &["diff", "patch"],
    &[
        "sh",
        "bash",
        "shell",
        "zsh",
        "fish",
        "ksh",
        "console",
        "shellsession",
        "terminal",
    ],
    &["html", "xml", "svg", "xhtml", "vue", "plist"],
    &[
        "yaml",
        "yml",
        "toml",
        "ini",
        "cfg",
        "conf",
        "properties",
        "editorconfig",
        "gitconfig",
        "desktop",
    ],
    // The generic lexer, one `Lang` each.
    &["rust", "rs"],
    &["c", "h"],
    &[
        "cpp",
        "c++",
        "cc",
        "cxx",
        "hpp",
        "hxx",
        "objc",
        "objective-c",
        "cuda",
        "glsl",
        "hlsl",
        "wgsl",
        "metal",
    ],
    &["csharp", "cs", "c#"],
    &["go", "golang"],
    &["java", "groovy"],
    &["kotlin", "kt", "kts"],
    &["swift"],
    &["dart"],
    &["scala"],
    &["javascript", "js", "jsx", "mjs", "cjs"],
    &["typescript", "ts", "tsx", "mts"],
    &["python", "py", "python3", "py3", "pyi", "gdscript"],
    &["ruby", "rb"],
    &["php"],
    &["lua"],
    &["sql", "psql", "mysql", "sqlite", "postgresql", "plsql"],
    &["css", "scss", "sass", "less"],
    &["json", "jsonc", "json5", "geojson"],
    &["haskell", "hs"],
    &["zig"],
    &["nix"],
    &["elixir", "ex", "exs"],
    &["make", "makefile", "mk"],
    &["dockerfile", "docker", "containerfile"],
];

pub fn all_langs() -> impl Iterator<Item = &'static str> {
    LANG_GROUPS.iter().flat_map(|g| g.iter().copied())
}
