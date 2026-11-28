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
        /// Number of lines in this chunk (`1..=CODE_CHUNK_LINES`).
        lines: u16,
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

impl Row {
    /// Index of the top-level `Block` in `Document::blocks` that owns this row.
    pub fn block(&self) -> usize {
        match *self {
            Row::Block(block)
            | Row::TableHeader { block, .. }
            | Row::TableBody { block, .. }
            | Row::ListItem { block, .. }
            | Row::CodeChunk { block, .. }
            | Row::QuoteHeader { block }
            | Row::QuoteBlock { block, .. } => block,
        }
    }
}

/// Where in a document a jump (an `#anchor`) lands: a top-level block and,
/// when the target is inside a block that's split into a row per item or
/// child (a list, a multi-block quote), which item or child holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub block: usize,
    /// Index into the list's items or the quote's child blocks.
    pub child: Option<usize>,
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
                            let lines = code[range.clone()].bytes().filter(|&b| b == b'\n').count()
                                as u16
                                + 1;
                            rows.push(Row::CodeChunk {
                                block: block_ix,
                                start: range.start,
                                end: range.end,
                                lines,
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

    /// The row that draws `place`: the item's row in a list, the child's
    /// row in a multi-block quote (after its callout title, if any), and the
    /// block's first row for anything else, including a quote that isn't
    /// split. `None` when the block doesn't exist; a child past the end
    /// stays on the block's last row.
    pub fn row_for_place(&self, place: Place) -> Option<usize> {
        let start = self.row_for_block(place.block)?;
        let Some(child) = place.child else {
            return Some(start);
        };
        let row = match self.rows[start] {
            Row::ListItem { .. } | Row::QuoteBlock { .. } => start + child,
            Row::QuoteHeader { .. } => start + 1 + child,
            _ => start,
        };
        Some(row.min(start + self.block_row_count(place.block) - 1))
    }

    /// Maps a list row index back to `(top_level_block_ix, offset_within_block)`.
    /// Returns `None` when `row_ix` is out of bounds.
    pub fn block_and_offset(&self, row_ix: usize) -> Option<(usize, usize)> {
        let block = self.rows.get(row_ix)?.block();
        Some((block, row_ix - self.block_start[block]))
    }

    fn block_row_count(&self, block_ix: usize) -> usize {
        let start = self.block_start[block_ix];
        let end = self
            .block_start
            .get(block_ix + 1)
            .copied()
            .unwrap_or(self.rows.len());
        end - start
    }

    /// Translates `old_row_ix` from `old` into `self` across a document reload,
    /// preserving the top-level block and within-block row offset even when
    /// earlier blocks grow or shrink. Sets `reset_offset` whenever the saved
    /// pixel offset inside the row is no longer valid (clamped row or a row
    /// that changed shape or shrank).
    pub fn remap_row(&self, old: &Rows, old_row_ix: usize) -> RemappedRow {
        if self.rows.is_empty() {
            return RemappedRow {
                item_ix: 0,
                reset_offset: true,
            };
        }
        let Some((block_ix, offset)) = old.block_and_offset(old_row_ix) else {
            return RemappedRow {
                item_ix: if old_row_ix == 0 {
                    0
                } else {
                    self.rows.len() - 1
                },
                reset_offset: true,
            };
        };
        if block_ix >= self.block_start.len() {
            return RemappedRow {
                item_ix: self.rows.len() - 1,
                reset_offset: true,
            };
        }
        let start = self.block_start[block_ix];
        let count = self.block_row_count(block_ix);
        let clamped_offset = offset.min(count.saturating_sub(1));
        let item_ix = start + clamped_offset;
        let reset_offset = clamped_offset != offset
            || row_shrank_or_changed(&old.rows[old_row_ix], &self.rows[item_ix]);
        RemappedRow {
            item_ix,
            reset_offset,
        }
    }
}

/// Result of remapping a scroll row across a document reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemappedRow {
    pub item_ix: usize,
    pub reset_offset: bool,
}

fn row_shrank_or_changed(old: &Row, new: &Row) -> bool {
    if std::mem::discriminant(old) != std::mem::discriminant(new) {
        return true;
    }
    matches!(
        (old, new),
        (
            Row::CodeChunk {
                lines: old_lines,
                ..
            },
            Row::CodeChunk {
                lines: new_lines,
                ..
            },
        ) if new_lines < old_lines
    )
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

/// Byte ranges of the line(s) in `code` most likely to be widest once shaped
/// (max character count, max UTF-8 byte length, max non-ASCII count, and max
/// tab-expanded columns). Measuring these seeds the shared horizontal scroll
/// width before any chunk is laid out; `render.rs` also grows that width
/// monotonically from actual shaped chunks.
pub fn widest_line_candidates(code: &str) -> [Range<usize>; 4] {
    let mut max_chars = (0usize, 0..0);
    let mut max_bytes = (0usize, 0..0);
    let mut max_non_ascii = (0usize, 0..0);
    let mut max_tab_cols = (0usize, 0..0);
    let mut start = 0;
    for line in code.split('\n') {
        let bytes = line.len();
        let end = start + bytes;
        let mut chars = 0usize;
        let mut non_ascii = 0usize;
        let mut tab_cols = 0usize;
        for c in line.chars() {
            chars += 1;
            if c == '\t' {
                tab_cols += 4;
            } else if c.is_ascii() {
                tab_cols += 1;
            } else {
                non_ascii += 1;
                tab_cols += 2;
            }
        }
        if chars > max_chars.0 {
            max_chars = (chars, start..end);
        }
        if bytes > max_bytes.0 {
            max_bytes = (bytes, start..end);
        }
        if non_ascii > max_non_ascii.0 {
            max_non_ascii = (non_ascii, start..end);
        }
        if tab_cols > max_tab_cols.0 {
            max_tab_cols = (tab_cols, start..end);
        }
        start = end + 1;
    }
    [max_chars.1, max_bytes.1, max_non_ascii.1, max_tab_cols.1]
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
                    lines: l0,
                    first: true,
                    last: false,
                },
                Row::CodeChunk {
                    block: 0,
                    start: s1,
                    end: e1,
                    lines: l1,
                    first: false,
                    last: true,
                },
                Row::Block(1),
            ) => {
                assert_eq!(*s0, 0);
                assert_eq!(*e1, code.len());
                assert_eq!(*l0, CODE_CHUNK_LINES as u16);
                assert_eq!(*l1, 1);
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
    fn places_map_to_the_row_of_their_item_or_child() {
        let doc = markdown::parse(concat!(
            "# Top\n\n",                 // 0: row 0
            "- a\n- b\n- c\n\n",         // 1: rows 1..=3
            "> [!TIP]\n> A\n>\n> B\n\n", // 2: header 4, rows 5..=6
            "> C\n>\n> D\n>\n> E\n\n",   // 3: rows 7..=9
            "> Single\n\n",              // 4: row 10
            "| x |\n|---|\n| 1 |\n\n",   // 5: rows 11..=12
            "End\n",                     // 6: row 13
        ));
        let rows = Rows::from_document(&doc);
        let at = |block, child| rows.row_for_place(Place { block, child });
        assert_eq!(at(0, None), Some(0));
        assert_eq!(at(1, None), Some(1));
        assert_eq!(at(1, Some(0)), Some(1));
        assert_eq!(at(1, Some(2)), Some(3));
        // The callout's title row comes first.
        assert_eq!(at(2, None), Some(4));
        assert_eq!(at(2, Some(0)), Some(5));
        assert_eq!(at(2, Some(1)), Some(6));
        assert_eq!(at(3, Some(0)), Some(7));
        assert_eq!(at(3, Some(2)), Some(9));
        // A quote with one block isn't split.
        assert_eq!(at(4, Some(0)), Some(10));
        // Other blocks ignore `child`.
        assert_eq!(at(5, Some(1)), Some(11));
        assert_eq!(at(6, Some(0)), Some(13));
        // A child past the end stays in its block; a missing block is None.
        assert_eq!(at(1, Some(9)), Some(3));
        assert_eq!(at(2, Some(9)), Some(6));
        assert_eq!(at(7, None), None);
        assert_eq!(at(7, Some(0)), None);
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

    #[test]
    fn block_and_offset_and_remap_preserve_block_across_table_edits() {
        let make_doc = |n: usize| {
            let mut s = String::from("| ID |\n|---|\n");
            for i in 0..n {
                s.push_str(&format!("| {i} |\n"));
            }
            s.push_str("\n## After Table\n\nTrailing paragraph.\n");
            Rows::from_document(&markdown::parse(&s))
        };

        let small = make_doc(5); // block 0: rows 0..=5, block 1 (heading): row 6, block 2: row 7
        let large = make_doc(105); // block 0: rows 0..=105, block 1 (heading): row 106, block 2: row 107

        // Every Row variant reports its owning block and offset.
        assert_eq!(small.block_and_offset(0), Some((0, 0)));
        assert_eq!(small.block_and_offset(5), Some((0, 5)));
        assert_eq!(small.block_and_offset(6), Some((1, 0)));
        assert_eq!(small.block_and_offset(7), Some((2, 0)));
        assert_eq!(small.block_and_offset(8), None);

        // Growing the preceding table from 5 to 105 rows keeps the scroll
        // position on `## After Table` (block 1 -> row 106, not row 6) and
        // preserves the pixel offset inside that heading.
        assert_eq!(
            large.remap_row(&small, 6),
            RemappedRow {
                item_ix: 106,
                reset_offset: false
            }
        );
        assert_eq!(
            large.remap_row(&small, 7),
            RemappedRow {
                item_ix: 107,
                reset_offset: false
            }
        );
        // Shrinking the table back returns to row 6 without resetting offset.
        assert_eq!(
            small.remap_row(&large, 106),
            RemappedRow {
                item_ix: 6,
                reset_offset: false
            }
        );
        // Staying inside the table preserves the row offset, and shrinking
        // clamps to the table's last row (resetting pixel offset) without
        // spilling into `## After Table`.
        assert_eq!(
            large.remap_row(&small, 3),
            RemappedRow {
                item_ix: 3,
                reset_offset: false
            }
        );
        assert_eq!(
            small.remap_row(&large, 50),
            RemappedRow {
                item_ix: 5,
                reset_offset: true
            }
        );
        // Same clamping when the shrinking block is NOT at block 0 (start > 0).
        let with_prefix = |n: usize| {
            let mut s = String::from("# Intro\n\n| ID |\n|---|\n");
            for i in 0..n {
                s.push_str(&format!("| {i} |\n"));
            }
            s.push_str("\n## Tail\n");
            Rows::from_document(&markdown::parse(&s))
        };
        let pref_large = with_prefix(20); // block 0: row 0, block 1: rows 1..=21, block 2: row 22
        let pref_small = with_prefix(3); // block 0: row 0, block 1: rows 1..=4, block 2: row 5
        assert_eq!(
            pref_small.remap_row(&pref_large, 15),
            RemappedRow {
                item_ix: 4,
                reset_offset: true
            }
        );
        assert_eq!(
            pref_small.remap_row(&pref_large, 22),
            RemappedRow {
                item_ix: 5,
                reset_offset: false
            }
        );
        // Top of document stays at 0; out-of-range (e.g. Bottom) clamps to end.
        assert_eq!(
            large.remap_row(&small, 0),
            RemappedRow {
                item_ix: 0,
                reset_offset: false
            }
        );
        assert_eq!(
            large.remap_row(&small, small.len()),
            RemappedRow {
                item_ix: large.len() - 1,
                reset_offset: true
            }
        );
        // Reloading into an empty doc or shorter block list clamps safely.
        let empty = Rows::from_document(&markdown::parse(""));
        assert_eq!(
            empty.remap_row(&small, 6),
            RemappedRow {
                item_ix: 0,
                reset_offset: true
            }
        );
        assert_eq!(
            small.remap_row(&empty, 0),
            RemappedRow {
                item_ix: 0,
                reset_offset: true
            }
        );
        let one_para = Rows::from_document(&markdown::parse("Only one block\n"));
        assert_eq!(
            one_para.remap_row(&small, 7),
            RemappedRow {
                item_ix: 0,
                reset_offset: true
            }
        );

        // Shrinking a split code block resets pixel offset whether the user
        // was on a removed chunk (chunk 1 -> row 0), chunk 0 (CodeChunk -> Block),
        // or a chunk that lost lines (64 -> 6 lines).
        let code_doc = |n: usize| {
            let mut s = String::from("```rust\n");
            for i in 0..n {
                s.push_str(&format!("let x = {i};\n"));
            }
            s.push_str("```\n\nAfter\n");
            Rows::from_document(&markdown::parse(&s))
        };
        let code_150 = code_doc(150);
        let code_200 = code_doc(200);
        let code_70 = code_doc(70);
        let code_1 = code_doc(1);
        assert_eq!(
            code_1.remap_row(&code_150, 1),
            RemappedRow {
                item_ix: 0,
                reset_offset: true
            }
        );
        assert_eq!(
            code_1.remap_row(&code_150, 0),
            RemappedRow {
                item_ix: 0,
                reset_offset: true
            }
        );
        assert_eq!(
            code_70.remap_row(&code_150, 1),
            RemappedRow {
                item_ix: 1,
                reset_offset: true
            }
        );
        assert_eq!(
            code_200.remap_row(&code_150, 1),
            RemappedRow {
                item_ix: 1,
                reset_offset: false
            }
        );
    }

    #[test]
    fn row_block_covers_all_variants() {
        assert_eq!(Row::Block(0).block(), 0);
        assert_eq!(
            Row::TableHeader {
                block: 1,
                last: true
            }
            .block(),
            1
        );
        assert_eq!(
            Row::TableBody {
                block: 2,
                row: 0,
                last: true
            }
            .block(),
            2
        );
        assert_eq!(
            Row::ListItem {
                block: 3,
                item: 0,
                last: true
            }
            .block(),
            3
        );
        assert_eq!(
            Row::CodeChunk {
                block: 4,
                start: 0,
                end: 1,
                lines: 1,
                first: true,
                last: true
            }
            .block(),
            4
        );
        assert_eq!(Row::QuoteHeader { block: 5 }.block(), 5);
        assert_eq!(
            Row::QuoteBlock {
                block: 6,
                child: 0,
                first: true,
                last: true
            }
            .block(),
            6
        );
    }

    #[test]
    fn widest_line_candidates_handles_ascii_bytes_emoji_tabs_and_ties() {
        assert_eq!(widest_line_candidates(""), [0..0, 0..0, 0..0, 0..0]);

        let ascii = "short\nlongest ascii line\nmid";
        let [by_chars, by_bytes, by_non_ascii, by_tabs] = widest_line_candidates(ascii);
        assert_eq!(&ascii[by_chars], "longest ascii line");
        assert_eq!(&ascii[by_bytes], "longest ascii line");
        assert_eq!(by_non_ascii, 0..0);
        assert_eq!(&ascii[by_tabs], "longest ascii line");

        // Regression from PR #16: 100 emoji (400 bytes, 100 non-ASCII) alongside
        // 110 ASCII `x` (wins chars) and 60 tabs (wins tab_cols). Both `by_bytes`
        // and `by_non_ascii` capture the emoji line.
        let emoji = "😀".repeat(100);
        let wide_ascii = "x".repeat(110);
        let tabs = "\t".repeat(60);
        let mixed = format!("{emoji}\n{wide_ascii}\n{tabs}");
        let [by_chars, by_bytes, by_non_ascii, by_tabs] = widest_line_candidates(&mixed);
        assert_eq!(&mixed[by_chars], wide_ascii);
        assert_eq!(&mixed[by_bytes], emoji);
        assert_eq!(&mixed[by_non_ascii], emoji);
        assert_eq!(&mixed[by_tabs], tabs);

        // Distinct non-ASCII winner when a shorter 2-byte line has more non-ASCII
        // chars than a longer ASCII line.
        let accents = "é".repeat(12);
        let ascii_long = "a".repeat(30);
        let sample = format!("{accents}\n{ascii_long}");
        let [by_chars, by_bytes, by_non_ascii, by_tabs] = widest_line_candidates(&sample);
        assert_eq!(&sample[by_chars], ascii_long);
        assert_eq!(&sample[by_bytes], ascii_long);
        assert_eq!(&sample[by_non_ascii], accents);
        assert_eq!(&sample[by_tabs], ascii_long);

        // Non-ASCII characters count as 2 columns in `tab_cols`: 4 CJK chars
        // (8 cols) beat 6 ASCII chars (6 cols).
        let cjk_vs_ascii = "abcdef\n漢字漢字";
        let [by_chars, _, _, by_tabs] = widest_line_candidates(cjk_vs_ascii);
        assert_eq!(&cjk_vs_ascii[by_chars], "abcdef");
        assert_eq!(&cjk_vs_ascii[by_tabs], "漢字漢字");

        // Ties keep the first widest line for every metric.
        let tied = "😀a\n😀b";
        let [by_chars, by_bytes, by_non_ascii, by_tabs] = widest_line_candidates(tied);
        assert_eq!(&tied[by_chars], "😀a");
        assert_eq!(&tied[by_bytes], "😀a");
        assert_eq!(&tied[by_non_ascii], "😀a");
        assert_eq!(&tied[by_tabs], "😀a");
    }
}
