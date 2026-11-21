//! Projects a parsed [`Document`]'s top-level blocks into virtualized list
//! rows so large tables, lists, code blocks and quotes scroll smoothly.
//!
//! GUI-free so the row layout, code chunking, highlight slicing and block→row
//! index mapping can be unit-tested without a window.

use std::ops::{Index, Range};

use crate::{
    highlight::Highlight,
    markdown::{Block, Document},
};

/// Maximum lines kept in a single list row for a fenced/indented code block.
/// Smaller code blocks stay one row so horizontal scrolling stays unified.
pub const CODE_CHUNK_LINES: usize = 64;

/// One row of the virtualized document list (`gpui::List`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A top-level block rendered as a single list row.
    Block(usize),
    /// The header row of a top-level `Block::Table`.
    TableHeader {
        block: usize,
        /// True when the table has no body rows, so the header also closes
        /// the bottom border and rounded corners.
        last: bool,
    },
    /// One body row of a top-level `Block::Table`.
    TableBody {
        block: usize,
        row: usize,
        last: bool,
    },
    /// One item of a top-level `Block::List`.
    ListItem {
        block: usize,
        item: usize,
        last: bool,
    },
    /// A line-aligned byte slice of a large top-level `Block::Code`.
    CodeChunk {
        block: usize,
        start: usize,
        end: usize,
        first: bool,
        last: bool,
    },
    /// The callout title of a multi-block top-level `Block::Quote`.
    QuoteHeader { block: usize },
    /// One child block of a multi-block top-level `Block::Quote`.
    QuoteBlock {
        block: usize,
        child: usize,
        first: bool,
        last: bool,
    },
}

/// Virtualized rows for a [`Document`], plus an O(1) map from top-level block
/// index to its first row index (for `#anchor` navigation).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Rows {
    rows: Vec<Row>,
    block_start: Vec<usize>,
}

impl Rows {
    pub fn from_document(doc: &Document) -> Self {
        Self::from_blocks(&doc.blocks)
    }

    pub fn from_blocks(blocks: &[Block]) -> Self {
        let mut rows = Vec::with_capacity(blocks.len());
        let mut block_start = Vec::with_capacity(blocks.len());
        for (block_ix, block) in blocks.iter().enumerate() {
            block_start.push(rows.len());
            match block {
                Block::Table { rows: body, .. } => {
                    rows.push(Row::TableHeader {
                        block: block_ix,
                        last: body.is_empty(),
                    });
                    for row in 0..body.len() {
                        rows.push(Row::TableBody {
                            block: block_ix,
                            row,
                            last: row + 1 == body.len(),
                        });
                    }
                }
                Block::List { items, .. } if !items.is_empty() => {
                    for item in 0..items.len() {
                        rows.push(Row::ListItem {
                            block: block_ix,
                            item,
                            last: item + 1 == items.len(),
                        });
                    }
                }
                Block::Code { code, .. } => {
                    if let Some(chunks) = code_chunks(code, CODE_CHUNK_LINES) {
                        let n = chunks.len();
                        for (i, range) in chunks.into_iter().enumerate() {
                            rows.push(Row::CodeChunk {
                                block: block_ix,
                                start: range.start,
                                end: range.end,
                                first: i == 0,
                                last: i + 1 == n,
                            });
                        }
                    } else {
                        rows.push(Row::Block(block_ix));
                    }
                }
                Block::Quote { callout, blocks } if blocks.len() > 1 => {
                    if callout.is_some() {
                        rows.push(Row::QuoteHeader { block: block_ix });
                    }
                    for child in 0..blocks.len() {
                        rows.push(Row::QuoteBlock {
                            block: block_ix,
                            child,
                            first: callout.is_none() && child == 0,
                            last: child + 1 == blocks.len(),
                        });
                    }
                }
                _ => rows.push(Row::Block(block_ix)),
            }
        }
        Self { rows, block_start }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    #[cfg(test)]
    pub fn iter(&self) -> std::slice::Iter<'_, Row> {
        self.rows.iter()
    }

    /// First list row belonging to top-level block `block_ix`.
    pub fn row_for_block(&self, block_ix: usize) -> Option<usize> {
        self.block_start.get(block_ix).copied()
    }
}

impl Index<usize> for Rows {
    type Output = Row;

    fn index(&self, ix: usize) -> &Self::Output {
        &self.rows[ix]
    }
}

/// Splits `code` into line-aligned byte ranges of at most `max_lines` lines
/// each, returning `None` when `code` fits in a single chunk.
///
/// Chunk boundaries fall on `'\n'` separators (excluded from both adjacent
/// chunks), so joining the slices with `'\n'` reproduces `code` verbatim.
pub fn code_chunks(code: &str, max_lines: usize) -> Option<Vec<Range<usize>>> {
    debug_assert!(max_lines > 0);
    let mut chunks = Vec::new();
    let mut chunk_start = 0;
    let mut lines = 0;
    for (i, b) in code.bytes().enumerate() {
        if b == b'\n' {
            lines += 1;
            if lines == max_lines {
                chunks.push(chunk_start..i);
                chunk_start = i + 1;
                lines = 0;
            }
        }
    }
    if chunks.is_empty() {
        return None;
    }
    chunks.push(chunk_start..code.len());
    Some(chunks)
}

/// Slices sorted, non-overlapping highlights to `range`, clamping spans that
/// cross chunk boundaries (such as multi-line comments or strings) and
/// shifting offsets to be relative to `range.start`.
pub fn slice_highlights(highlights: &[Highlight], range: Range<usize>) -> Vec<Highlight> {
    if range.is_empty() {
        return Vec::new();
    }
    let first = highlights.partition_point(|h| h.range.end <= range.start);
    let mut out = Vec::new();
    for h in &highlights[first..] {
        if h.range.start >= range.end {
            break;
        }
        out.push(Highlight {
            range: (h.range.start.max(range.start) - range.start)
                ..(h.range.end.min(range.end) - range.start),
            tok: h.tok,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        highlight::Tok,
        markdown::{self, Callout, RichText},
    };

    #[test]
    fn empty_document() {
        let rows = Rows::from_document(&markdown::parse(""));
        assert!(rows.is_empty());
        assert_eq!(rows.len(), 0);
        assert_eq!(rows.row_for_block(0), None);
    }

    #[test]
    fn splits_tables_into_header_and_body_rows() {
        let doc = markdown::parse("# Top\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\nAfter\n");
        let rows = Rows::from_document(&doc);
        assert_eq!(
            rows.iter().cloned().collect::<Vec<_>>(),
            [
                Row::Block(0),
                Row::TableHeader {
                    block: 1,
                    last: false,
                },
                Row::TableBody {
                    block: 1,
                    row: 0,
                    last: false,
                },
                Row::TableBody {
                    block: 1,
                    row: 1,
                    last: true,
                },
                Row::Block(2),
            ]
        );
        assert_eq!(rows.row_for_block(0), Some(0));
        assert_eq!(rows.row_for_block(1), Some(1));
        assert_eq!(rows.row_for_block(2), Some(4));
        assert_eq!(rows.row_for_block(3), None);
    }

    #[test]
    fn header_only_table_marks_header_as_last() {
        let doc = markdown::parse("| A | B |\n|---|---|\n");
        let rows = Rows::from_document(&doc);
        assert_eq!(
            rows.iter().cloned().collect::<Vec<_>>(),
            [Row::TableHeader {
                block: 0,
                last: true,
            }]
        );
    }

    #[test]
    fn splits_lists_per_item_and_falls_back_when_empty() {
        let doc = markdown::parse("- one\n- two\n- three\n\n## After\n");
        let rows = Rows::from_document(&doc);
        assert_eq!(
            rows.iter().cloned().collect::<Vec<_>>(),
            [
                Row::ListItem {
                    block: 0,
                    item: 0,
                    last: false,
                },
                Row::ListItem {
                    block: 0,
                    item: 1,
                    last: false,
                },
                Row::ListItem {
                    block: 0,
                    item: 2,
                    last: true,
                },
                Row::Block(1),
            ]
        );
        assert_eq!(rows.row_for_block(1), Some(3));

        // Synthetic empty list falls back to a single Block row.
        let empty_list = [Block::List {
            start: None,
            items: Vec::new(),
        }];
        let rows = Rows::from_blocks(&empty_list);
        assert_eq!(rows.iter().cloned().collect::<Vec<_>>(), [Row::Block(0)]);
    }

    #[test]
    fn splits_only_large_code_blocks() {
        let small: String = (0..CODE_CHUNK_LINES)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let doc = markdown::parse(&format!("```rust\n{small}\n```\n"));
        let rows = Rows::from_document(&doc);
        assert_eq!(rows.iter().cloned().collect::<Vec<_>>(), [Row::Block(0)]);

        let large: String = (0..=CODE_CHUNK_LINES)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let doc = markdown::parse(&format!("```rust\n{large}\n```\n\n## End\n"));
        let rows = Rows::from_document(&doc);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.row_for_block(0), Some(0));
        assert_eq!(rows.row_for_block(1), Some(2));
        let Block::Code { code, .. } = &doc.blocks[0] else {
            panic!("expected code block");
        };
        match (&rows[0], &rows[1], &rows[2]) {
            (
                Row::CodeChunk {
                    block: 0,
                    start: s0,
                    end: e0,
                    first: true,
                    last: false,
                },
                Row::CodeChunk {
                    block: 0,
                    start: s1,
                    end: e1,
                    first: false,
                    last: true,
                },
                Row::Block(1),
            ) => {
                assert_eq!(*s0, 0);
                assert_eq!(*e1, code.len());
                assert_eq!(format!("{}\n{}", &code[*s0..*e0], &code[*s1..*e1]), *code);
                assert_eq!(code[*s0..*e0].lines().count(), CODE_CHUNK_LINES);
                assert_eq!(&code[*s1..*e1], format!("line {CODE_CHUNK_LINES}"));
            }
            other => panic!("unexpected rows: {other:?}"),
        }
    }

    #[test]
    fn splits_multi_block_quotes_with_and_without_callout() {
        let doc = markdown::parse(
            "> Single paragraph\n\n> [!NOTE]\n> One\n\n> First\n>\n> Second\n\n> [!WARNING]\n> A\n>\n> B\n\n## End\n",
        );
        let rows = Rows::from_document(&doc);
        assert_eq!(
            rows.iter().cloned().collect::<Vec<_>>(),
            [
                // Single-block quote stays one row.
                Row::Block(0),
                // Single-block callout stays one row.
                Row::Block(1),
                // Multi-block plain quote splits per child block.
                Row::QuoteBlock {
                    block: 2,
                    child: 0,
                    first: true,
                    last: false,
                },
                Row::QuoteBlock {
                    block: 2,
                    child: 1,
                    first: false,
                    last: true,
                },
                // Multi-block callout gets a header row + per-child rows.
                Row::QuoteHeader { block: 3 },
                Row::QuoteBlock {
                    block: 3,
                    child: 0,
                    first: false,
                    last: false,
                },
                Row::QuoteBlock {
                    block: 3,
                    child: 1,
                    first: false,
                    last: true,
                },
                Row::Block(4),
            ]
        );
        assert_eq!(rows.row_for_block(0), Some(0));
        assert_eq!(rows.row_for_block(1), Some(1));
        assert_eq!(rows.row_for_block(2), Some(2));
        assert_eq!(rows.row_for_block(3), Some(4));
        assert_eq!(rows.row_for_block(4), Some(7));

        // Synthetic 3-block callout checks middle block flags.
        let blocks = [Block::Quote {
            callout: Some(Callout::Tip),
            blocks: vec![
                Block::Paragraph(RichText::default()),
                Block::Paragraph(RichText::default()),
                Block::Paragraph(RichText::default()),
            ],
        }];
        let rows = Rows::from_blocks(&blocks);
        assert_eq!(
            rows.iter().cloned().collect::<Vec<_>>(),
            [
                Row::QuoteHeader { block: 0 },
                Row::QuoteBlock {
                    block: 0,
                    child: 0,
                    first: false,
                    last: false,
                },
                Row::QuoteBlock {
                    block: 0,
                    child: 1,
                    first: false,
                    last: false,
                },
                Row::QuoteBlock {
                    block: 0,
                    child: 2,
                    first: false,
                    last: true,
                },
            ]
        );
    }

    #[test]
    fn code_chunks_preserves_exact_content_and_empty_lines() {
        assert_eq!(code_chunks("", 2), None);
        assert_eq!(code_chunks("a", 2), None);
        assert_eq!(code_chunks("a\nb", 2), None);

        for src in [
            "a\nb\nc",
            "a\nb\n",
            "a\nb\nc\nd",
            "a\nb\nc\nd\ne",
            "\n\n",
            "héllo\nwörld\n🚀\nend",
        ] {
            let chunks = code_chunks(src, 2).expect(src);
            let joined: Vec<&str> = chunks.iter().map(|r| &src[r.clone()]).collect();
            assert_eq!(joined.join("\n"), src, "round-trip for {src:?}");
            for piece in &joined {
                assert!(
                    piece.bytes().filter(|&b| b == b'\n').count() < 2,
                    "chunk too many lines: {piece:?}"
                );
            }
        }
        assert_eq!(code_chunks("a\nb\n", 2), Some(vec![0..3, 4..4]));
        assert_eq!(
            code_chunks("a\nb\nc\nd\ne", 2),
            Some(vec![0..3, 4..7, 8..9])
        );
    }

    #[test]
    fn slice_highlights_clamps_and_shifts_boundaries() {
        let hs = vec![
            Highlight {
                range: 0..3,
                tok: Tok::Keyword,
            },
            Highlight {
                range: 3..5, // touches start=5
                tok: Tok::Type,
            },
            Highlight {
                range: 8..10, // strictly inside 5..12
                tok: Tok::Number,
            },
            Highlight {
                range: 10..15, // overlaps end=12
                tok: Tok::String,
            },
            Highlight {
                range: 16..18, // strictly after
                tok: Tok::Keyword,
            },
        ];
        assert_eq!(
            slice_highlights(&hs, 5..12),
            vec![
                Highlight {
                    range: 3..5,
                    tok: Tok::Number,
                },
                Highlight {
                    range: 5..7,
                    tok: Tok::String,
                },
            ]
        );
        // Span crossing start boundary, span touching end boundary.
        let crossing = vec![
            Highlight {
                range: 2..7,
                tok: Tok::Comment,
            },
            Highlight {
                range: 7..10,
                tok: Tok::Keyword,
            },
            Highlight {
                range: 10..13,
                tok: Tok::Type,
            },
        ];
        assert_eq!(
            slice_highlights(&crossing, 5..10),
            vec![
                Highlight {
                    range: 0..2,
                    tok: Tok::Comment,
                },
                Highlight {
                    range: 2..5,
                    tok: Tok::Keyword,
                },
            ]
        );
        // Span covering the entire slice.
        let spanning = vec![Highlight {
            range: 0..20,
            tok: Tok::Comment,
        }];
        assert_eq!(
            slice_highlights(&spanning, 5..10),
            vec![Highlight {
                range: 0..5,
                tok: Tok::Comment,
            }]
        );
        // Empty slice returns empty.
        assert!(slice_highlights(&spanning, 5..5).is_empty());
    }
}
