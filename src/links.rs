//! What a link in a document points to. GUI-free, so the routing rules can be
//! unit-tested; `render::open_link` turns a [`LinkTarget`] into an action.

use std::path::{Path, PathBuf};

use crate::{markdown::Block, rows::Place};

/// Where clicking a link goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// `#anchor`: a heading in the current tab (percent-decoded).
    Anchor(String),
    /// `scheme://…` or `mailto:`: handed to the system as is.
    External(String),
    /// A Markdown file, opened in a tab, plus an optional anchor in it.
    Markdown {
        path: PathBuf,
        anchor: Option<String>,
    },
    /// Any other local file, opened with the system handler.
    File(PathBuf),
}

/// File extensions opened in a tab instead of the system handler.
const MARKDOWN_EXTENSIONS: [&str; 5] = ["md", "markdown", "mdx", "mdown", "mkd"];

/// Decides what clicking `url` does, for a document in `base`.
pub fn classify_link(base: &Path, url: &str) -> LinkTarget {
    if let Some(anchor) = url.strip_prefix('#') {
        return LinkTarget::Anchor(percent_decode(anchor));
    }
    if url.contains("://") || url.starts_with("mailto:") {
        return LinkTarget::External(url.to_owned());
    }
    let (path, anchor) = match url.split_once('#') {
        Some((p, a)) => (p, Some(a)),
        None => (url, None),
    };
    let path = base.join(percent_decode(path));
    if is_markdown(&path) {
        LinkTarget::Markdown {
            path,
            anchor: anchor.map(percent_decode),
        }
    } else {
        LinkTarget::File(path)
    }
}

/// Whether `path` has a Markdown extension (case-insensitive).
pub fn is_markdown(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        MARKDOWN_EXTENSIONS
            .iter()
            .any(|m| e.eq_ignore_ascii_case(m))
    })
}

/// Percent-encoded URL for an absolute local file, including Windows drive/UNC paths.
#[cfg(any(unix, test))]
pub fn file_url(path: &Path) -> String {
    reqwest::Url::from_file_path(path)
        .expect("local link paths are absolute")
        .into()
}

/// Decodes `%XX` escapes. Invalid escapes are kept as they are, and bytes that
/// don't form valid UTF-8 become U+FFFD.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (
                (b[i + 1] as char).to_digit(16),
                (b[i + 2] as char).to_digit(16),
            )
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where the heading whose slug is `anchor` is drawn, compared
/// case-insensitively (slugs are lowercase). Headings nested in quotes, list
/// items and footnotes count too (slugs are unique, nested ones included).
/// A bare `#` (an empty anchor) matches nothing, not a heading without
/// letters or digits.
pub fn heading_place(blocks: &[Block], anchor: &str) -> Option<Place> {
    if anchor.is_empty() {
        return None;
    }
    let anchor = anchor.to_lowercase();
    find(
        blocks,
        &|b| matches!(b, Block::Heading { slug, .. } if *slug == anchor),
    )
}

/// Where the definition of the footnote `label` is drawn: the first
/// `Block::Footnote` whose label matches, at any depth (pulldown-cmark also
/// accepts definitions inside quotes and list items). Labels are compared
/// the way pulldown-cmark pairs references with definitions, ignoring case
/// with full Unicode case folding (`ß` matches `SS`, `ς` matches `Σ`);
/// pulldown-cmark has already collapsed their whitespace. Footnotes and
/// headings are looked up separately, so `[^why]` never lands on a
/// `# why` heading, nor `#why` on the footnote.
pub fn footnote_place(blocks: &[Block], label: &str) -> Option<Place> {
    find(
        blocks,
        &|b| matches!(b, Block::Footnote { label: l, .. } if fold(l).eq(fold(label))),
    )
}

/// `s` case-folded, close enough to Unicode's full folding for labels:
/// uppercasing first turns `ß` into `SS` and `ς` into `Σ`.
fn fold(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
}

/// The first block that `is` picks, in document order, at any depth, as the
/// [`Place`] whose row draws it. A list or multi-block quote is split into
/// a row per item or child (see `rows.rs`), so a target inside one lands on
/// that item's or child's row; anything deeper is drawn inside that row.
fn find(blocks: &[Block], is: &dyn Fn(&Block) -> bool) -> Option<Place> {
    blocks.iter().enumerate().find_map(|(block, b)| {
        let child = match b {
            _ if is(b) => None,
            Block::Quote { blocks, .. } => Some(blocks.iter().position(|c| contains(c, is))?),
            Block::List { items, .. } => Some(
                items
                    .iter()
                    .position(|i| i.blocks.iter().any(|c| contains(c, is)))?,
            ),
            // A footnote's body is drawn in the footnote's row.
            _ if contains(b, is) => None,
            _ => return None,
        };
        Some(Place { block, child })
    })
}

/// Whether `b`, or a block nested in it, is one `is` picks.
fn contains(b: &Block, is: &dyn Fn(&Block) -> bool) -> bool {
    is(b)
        || match b {
            Block::Quote { blocks, .. } | Block::Footnote { blocks, .. } => {
                blocks.iter().any(|c| contains(c, is))
            }
            Block::List { items, .. } => items
                .iter()
                .flat_map(|i| &i.blocks)
                .any(|c| contains(c, is)),
            _ => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown;
    use LinkTarget::*;

    const BASE: &str = "/docs/dir";

    fn link(url: &str) -> LinkTarget {
        classify_link(Path::new(BASE), url)
    }

    fn md(path: &str, anchor: Option<&str>) -> LinkTarget {
        Markdown {
            path: PathBuf::from(path),
            anchor: anchor.map(str::to_owned),
        }
    }

    #[test]
    fn anchors() {
        assert_eq!(link("#intro"), Anchor("intro".into()));
        assert_eq!(link("#%C3%A9t%C3%A9"), Anchor("été".into()));
        assert_eq!(link("#a%20b"), Anchor("a b".into()));
        // Kept as written: case is folded later, by `heading_place`.
        assert_eq!(link("#Intro"), Anchor("Intro".into()));
        assert_eq!(link("#"), Anchor(String::new()));
        assert_eq!(link("##x"), Anchor("#x".into()));
        // An anchor isn't split again at a second '#'.
        assert_eq!(link("#a#b"), Anchor("a#b".into()));
        // An anchor that looks like a URL is still an anchor.
        assert_eq!(link("#http://x"), Anchor("http://x".into()));
    }

    #[test]
    fn external() {
        for url in [
            "https://example.com",
            "http://example.com/a.md",
            "https://example.com/a.md#sec",
            "ftp://host/file",
            "file:///etc/passwd",
            "mailto:me@example.com",
            "mailto:",
            // Anything containing "://" counts, even after a relative path.
            "docs/x://y.md",
            "https://example.com/%20a?q=1#frag",
        ] {
            assert_eq!(link(url), External(url.into()), "{url}");
        }
    }

    #[test]
    fn other_schemes_are_treated_as_relative_paths() {
        // Pinned current behavior: only "://" and "mailto:" are external.
        assert_eq!(link("tel:123"), File("/docs/dir/tel:123".into()));
        assert_eq!(
            link("data:text/plain,hi"),
            File("/docs/dir/data:text/plain,hi".into())
        );
        assert_eq!(link("MAILTO:me@x"), File("/docs/dir/MAILTO:me@x".into()));
    }

    #[test]
    fn markdown_files() {
        assert_eq!(link("b.md"), md("/docs/dir/b.md", None));
        assert_eq!(link("./b.md"), md("/docs/dir/./b.md", None));
        assert_eq!(link("../b.MD#Sec"), md("/docs/dir/../b.MD", Some("Sec")));
        assert_eq!(link("sub/c.markdown"), md("/docs/dir/sub/c.markdown", None));
        assert_eq!(link("x.mdx"), md("/docs/dir/x.mdx", None));
        assert_eq!(link("x.mdown"), md("/docs/dir/x.mdown", None));
        assert_eq!(link("x.MkD"), md("/docs/dir/x.MkD", None));
        assert_eq!(link("/abs/b.md"), md("/abs/b.md", None));
        assert_eq!(link("a%20b.md"), md("/docs/dir/a b.md", None));
        assert_eq!(link("b.md#"), md("/docs/dir/b.md", Some("")));
        assert_eq!(link("b.md#%C3%A9%20x"), md("/docs/dir/b.md", Some("é x")));
        assert_eq!(link("b.md#a#b"), md("/docs/dir/b.md", Some("a#b")));
        assert_eq!(link("café.md"), md("/docs/dir/café.md", None));
        assert_eq!(link("caf%C3%A9.md"), md("/docs/dir/café.md", None));
        // `%23` is a literal '#' in the file name, not an anchor.
        assert_eq!(link("a%23b.md"), md("/docs/dir/a#b.md", None));
    }

    #[test]
    fn other_files() {
        assert_eq!(link("file.png"), File("/docs/dir/file.png".into()));
        assert_eq!(link("dir/"), File("/docs/dir/dir/".into()));
        assert_eq!(link("README"), File("/docs/dir/README".into()));
        assert_eq!(link("a.md.bak"), File("/docs/dir/a.md.bak".into()));
        assert_eq!(link(".md"), File("/docs/dir/.md".into()));
        assert_eq!(link("x.txt#L10"), File("/docs/dir/x.txt".into()));
        // An empty link points at the document's folder.
        assert_eq!(link(""), File("/docs/dir".into()));
    }

    #[test]
    fn query_strings_are_not_stripped() {
        // Pinned current behavior: "?" is part of the file name, so a
        // Markdown link with a query string goes to the system handler.
        assert_eq!(link("b.md?plain=1"), File("/docs/dir/b.md?plain=1".into()));
        assert_eq!(
            link("b.md?x#y"),
            File("/docs/dir/b.md?x".into()),
            "the anchor is dropped for non-Markdown targets"
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_urls() {
        assert_eq!(file_url(Path::new("/a/b.png")), "file:///a/b.png");
        assert_eq!(
            file_url(Path::new("/a b/c#d.png")),
            "file:///a%20b/c%23d.png"
        );
        assert_eq!(file_url(Path::new("/a%20b?.png")), "file:///a%2520b%3F.png");
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_and_urls() {
        let base = Path::new(r"C:\docs");
        assert_eq!(
            classify_link(base, r"D:\other\a.md#intro"),
            Markdown {
                path: PathBuf::from(r"D:\other\a.md"),
                anchor: Some("intro".into())
            }
        );
        assert_eq!(
            classify_link(base, r"\\server\share\a.md"),
            Markdown {
                path: PathBuf::from(r"\\server\share\a.md"),
                anchor: None
            }
        );
        assert_eq!(
            file_url(Path::new(r"C:\a b\c#d.png")),
            "file:///C:/a%20b/c%23d.png"
        );
        assert_eq!(
            file_url(Path::new(r"\\server\share\a b.png")),
            "file://server/share/a%20b.png"
        );
        assert_eq!(
            file_url(Path::new(r"\\?\C:\a b.png")),
            "file:///C:/a%20b.png"
        );
    }

    #[test]
    fn markdown_extension() {
        for p in [
            "a.md",
            "a.MD",
            "a.Markdown",
            "a.mdx",
            "a.mdown",
            "a.mkd",
            "/x/.y/z.md",
        ] {
            assert!(is_markdown(Path::new(p)), "{p}");
        }
        for p in [
            "a", "a.txt", "a.mdd", "a.m", "md", ".md", "a.md/", "a.md.txt", "a.mkdn", "a.",
        ] {
            assert_eq!(is_markdown(Path::new(p)), p == "a.md/", "{p}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_extension_is_not_markdown() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        let p = Path::new(OsStr::from_bytes(b"a.m\xffd"));
        assert!(!is_markdown(p));
    }

    #[test]
    fn percent_decoding() {
        let cases = [
            ("", ""),
            ("plain", "plain"),
            ("a%20b", "a b"),
            ("%41%42%43", "ABC"),
            ("%4a%4A", "JJ"),
            ("%C3%A9", "é"),
            ("100%", "100%"),
            ("%", "%"),
            ("%%", "%%"),
            ("%4", "%4"),
            ("a%4", "a%4"),
            ("%zz", "%zz"),
            ("%g0", "%g0"),
            ("%0g", "%0g"),
            ("%%41", "%A"),
            ("%2541", "%41"),
            ("%41%", "A%"),
            ("%410", "A0"),
            ("é%20ü", "é ü"),
            ("%é", "%é"),
            ("%4é", "%4é"),
            ("%FF", "\u{FFFD}"),
            ("%C3", "\u{FFFD}"),
            ("%C3%28", "\u{FFFD}("),
            ("%00", "\0"),
            ("%2F", "/"),
            ("+", "+"),
            ("%2B", "+"),
        ];
        for (input, want) in cases {
            assert_eq!(percent_decode(input), want, "{input:?}");
        }
    }

    #[test]
    fn percent_decoding_at_end() {
        // The escape is decoded right up to the last byte.
        assert_eq!(percent_decode("ab%41"), "abA");
        assert_eq!(percent_decode("ab%4"), "ab%4");
    }

    fn top(block: usize) -> Option<Place> {
        Some(Place { block, child: None })
    }

    fn child(block: usize, child: usize) -> Option<Place> {
        Some(Place {
            block,
            child: Some(child),
        })
    }

    #[test]
    fn anchor_lookup() {
        let doc = markdown::parse(
            "Intro text\n\n# Hello World\n\n## Café Ünïcode\n\n> # Nested\n\n### Hello World\n\n# A_b-c 1\n",
        );
        let blocks = &doc.blocks;
        assert_eq!(heading_place(blocks, "hello-world"), top(1));
        assert_eq!(heading_place(blocks, "Hello-World"), top(1));
        assert_eq!(heading_place(blocks, "HELLO-WORLD"), top(1));
        assert_eq!(heading_place(blocks, "café-ünïcode"), top(2));
        assert_eq!(heading_place(blocks, "CAFÉ-ÜNÏCODE"), top(2));
        assert_eq!(heading_place(blocks, "a_b-c-1"), top(5));
        // Nested headings are targets too (issue #21): this one is the
        // only child of a quote.
        assert_eq!(heading_place(blocks, "nested"), child(3, 0));
        assert_eq!(heading_place(blocks, "hello world"), None);
        assert_eq!(heading_place(blocks, "intro-text"), None);
        assert_eq!(heading_place(blocks, ""), None);
        assert_eq!(heading_place(&[], "x"), None);
    }

    #[test]
    fn repeated_headings_are_reached_by_suffixed_anchors() {
        // Issue #20.
        let doc = markdown::parse(
            "# Setup 1\n\n# Setup\n\n## Setup\n\n### setup\n\n## Added\n\n## Added\n",
        );
        let blocks = &doc.blocks;
        assert_eq!(heading_place(blocks, "setup-1"), top(0));
        assert_eq!(heading_place(blocks, "setup"), top(1));
        assert_eq!(heading_place(blocks, "setup-2"), top(2));
        assert_eq!(heading_place(blocks, "Setup-3"), top(3));
        assert_eq!(heading_place(blocks, "setup-4"), None);
        assert_eq!(heading_place(blocks, "added"), top(4));
        assert_eq!(heading_place(blocks, "added-1"), top(5));
    }

    #[test]
    fn headings_nested_in_quotes_lists_and_footnotes_are_found() {
        // Issue #21: only top-level headings used to be anchor targets.
        let doc = markdown::parse(concat!(
            "# Top\n\n",                                       // 0
            "> First\n>\n> ## In Quote\n\n",                   // 1, child 1
            "- a\n- b\n- ## In List\n- d\n\n",                 // 2, item 2
            "1. x\n2. y\n   > - z\n   >   ### Deep\n\n",       // 3, item 1
            "> [!NOTE]\n> One\n>\n> - ## In Callout List\n\n", // 4, child 1
            "Ref[^n].\n\n",                                    // 5
            "[^n]: ## In Footnote\n\n    > #### Deeper\n\n",   // 6
            "> > ## Doubly Quoted\n\n",                        // 7, child 0
            "## Setup\n\n- ## Setup\n",                        // 8, 9 item 0
        ));
        let blocks = &doc.blocks;
        assert_eq!(heading_place(blocks, "top"), top(0));
        assert_eq!(heading_place(blocks, "in-quote"), child(1, 1));
        assert_eq!(heading_place(blocks, "in-list"), child(2, 2));
        assert_eq!(heading_place(blocks, "deep"), child(3, 1));
        assert_eq!(heading_place(blocks, "in-callout-list"), child(4, 1));
        assert_eq!(heading_place(blocks, "in-footnote"), top(6));
        assert_eq!(heading_place(blocks, "deeper"), top(6));
        assert_eq!(heading_place(blocks, "doubly-quoted"), child(7, 0));
        // Repeats are numbered across nesting, so each is reachable.
        assert_eq!(heading_place(blocks, "setup"), top(8));
        assert_eq!(heading_place(blocks, "setup-1"), child(9, 0));
        // Text that isn't a heading isn't a target.
        assert_eq!(heading_place(blocks, "first"), None);
        assert_eq!(heading_place(blocks, "z"), None);
    }

    #[test]
    fn footnote_lookup() {
        // Issue #23.
        let doc = markdown::parse(concat!(
            "Refs[^a] [^Nested] [^ß] [^ς] [^x y].\n\n",    // 0
            "[^a]: First.\n\n",                            // 1
            "- one\n- two\n\n  [^nested]: In a list.\n\n", // 2, item 1
            "> Quote\n>\n> [^ss]: In a quote.\n\n",        // 3, child 1
            "[^σ]: Sigma.\n\n",                            // 4
            "[^a]: Repeated.\n\n",                         // 5
            "[^x  y]: Spaces.\n",                          // 6
        ));
        let blocks = &doc.blocks;
        assert_eq!(footnote_place(blocks, "a"), top(1), "the first one wins");
        assert_eq!(footnote_place(blocks, "A"), top(1));
        assert_eq!(footnote_place(blocks, "Nested"), child(2, 1));
        assert_eq!(footnote_place(blocks, "ß"), child(3, 1));
        assert_eq!(footnote_place(blocks, "SS"), child(3, 1));
        assert_eq!(footnote_place(blocks, "ς"), top(4));
        assert_eq!(footnote_place(blocks, "Σ"), top(4));
        // pulldown-cmark collapses the definition's whitespace.
        assert_eq!(footnote_place(blocks, "x y"), top(6));
        // No definition: nothing to jump to.
        assert_eq!(footnote_place(blocks, "missing"), None);
        assert_eq!(footnote_place(blocks, "s"), None);
        assert_eq!(footnote_place(blocks, ""), None);
        assert_eq!(footnote_place(&[], "a"), None);
    }

    #[test]
    fn footnotes_and_headings_do_not_shadow_each_other() {
        let doc = markdown::parse(concat!(
            "Ref[^why] [^fn-1].\n\n", // 0
            "# Why\n\n",              // 1
            "[^why]: Because.\n\n",   // 2
            "# fn-1\n\n",             // 3
            "[^fn-1]: Note.\n\n",     // 4
            "[^only]: Note.\n",       // 5
        ));
        let blocks = &doc.blocks;
        assert_eq!(heading_place(blocks, "why"), top(1));
        assert_eq!(footnote_place(blocks, "why"), top(2));
        assert_eq!(heading_place(blocks, "fn-1"), top(3));
        assert_eq!(footnote_place(blocks, "fn-1"), top(4));
        // A footnote isn't a heading anchor, and a heading isn't a footnote.
        assert_eq!(heading_place(blocks, "only"), None);
        assert_eq!(footnote_place(blocks, "Why"), top(2));
    }

    #[test]
    fn bare_hash_does_not_match_an_empty_slug() {
        let doc = markdown::parse("Intro\n\n# ???\n\n# !!!\n");
        assert_eq!(heading_place(&doc.blocks, ""), None);
        // The repeat is still reachable, as on GitHub.
        assert_eq!(heading_place(&doc.blocks, "-1"), top(2));
    }
}
