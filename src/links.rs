//! What a link in a document points to. GUI-free, so the routing rules can be
//! unit-tested; `render::open_link` turns a [`LinkTarget`] into an action.

use std::path::{Path, PathBuf};

use crate::markdown::Block;

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

/// Index of the top-level heading whose slug is `anchor`, compared
/// case-insensitively (slugs are lowercase).
pub fn heading_index(blocks: &[Block], anchor: &str) -> Option<usize> {
    let anchor = anchor.to_lowercase();
    blocks
        .iter()
        .position(|b| matches!(b, Block::Heading { slug, .. } if *slug == anchor))
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
        // Kept as written: case is folded later, by `heading_index`.
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

    #[test]
    fn anchor_lookup() {
        let doc = markdown::parse(
            "Intro text\n\n# Hello World\n\n## Café Ünïcode\n\n> # Nested\n\n### Hello World\n\n# A_b-c 1\n",
        );
        let blocks = &doc.blocks;
        assert_eq!(heading_index(blocks, "hello-world"), Some(1));
        assert_eq!(heading_index(blocks, "Hello-World"), Some(1));
        assert_eq!(heading_index(blocks, "HELLO-WORLD"), Some(1));
        assert_eq!(heading_index(blocks, "café-ünïcode"), Some(2));
        assert_eq!(heading_index(blocks, "CAFÉ-ÜNÏCODE"), Some(2));
        assert_eq!(heading_index(blocks, "a_b-c-1"), Some(5));
        // Only top-level headings are targets.
        assert_eq!(heading_index(blocks, "nested"), None);
        assert_eq!(heading_index(blocks, "hello world"), None);
        assert_eq!(heading_index(blocks, "intro-text"), None);
        assert_eq!(heading_index(blocks, ""), None);
        assert_eq!(heading_index(&[], "x"), None);
    }
}
