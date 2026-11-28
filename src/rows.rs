//! Projects a parsed [`Document`]'s top-level blocks into virtualized list
//! rows so large tables, lists, code blocks and quotes scroll smoothly.
//!
//! GUI-free so the row layout, code chunking, highlight slicing and block→row
//! index mapping can be unit-tested without a window.

use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    ops::{Index, Range},
};

use crate::{
    highlight::Highlight,
    markdown::{Block, Callout, Document},
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

    /// Translates `old_row_ix` from `old` (the rows of `old_blocks`) into
    /// `self` (the rows of `blocks`) across a document reload.
    ///
    /// The row's top-level block is found in the new document by content
    /// (see [`find_moved`]), so inserting or deleting blocks above it doesn't
    /// move the reader to another section, and editing the block itself
    /// stays on it. Inside a table, list or quote that changed, the row is
    /// matched the same way among its body rows, items or child blocks.
    /// Sets `reset_offset` whenever the saved pixel offset inside the row is
    /// no longer valid (the row was removed or clamped, or changed shape or
    /// shrank). An offset past the end of an edited row is left to the list,
    /// which clamps it once the row is measured.
    pub fn remap_row(
        &self,
        blocks: &[Block],
        old: &Rows,
        old_blocks: &[Block],
        old_row_ix: usize,
    ) -> RemappedRow {
        if self.rows.is_empty() {
            return RemappedRow {
                item_ix: 0,
                reset_offset: true,
            };
        }
        let last = self.rows.len() - 1;
        let Some((old_block, offset)) = old.block_and_offset(old_row_ix) else {
            return RemappedRow {
                item_ix: if old_row_ix == 0 { 0 } else { last },
                reset_offset: true,
            };
        };
        let Some(moved) = find_moved(old_blocks, blocks, old_block) else {
            return RemappedRow {
                item_ix: last,
                reset_offset: true,
            };
        };
        let (block_ix, sub, removed) = match moved {
            Moved::Same(b) => (b, offset, false),
            Moved::Edited(b) => match sub_row(&old_blocks[old_block], &blocks[b], offset) {
                Some(Moved::Same(sub) | Moved::Edited(sub)) => (b, sub, false),
                Some(Moved::Removed(sub)) => (b, sub, true),
                None => (b, offset, false),
            },
            Moved::Removed(b) => (b, 0, true),
        };
        let start = self.block_start[block_ix];
        let count = self.block_row_count(block_ix);
        let clamped = sub.min(count - 1);
        let item_ix = start + clamped;
        let reset_offset = removed
            || clamped != sub
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

/// Where an element of an old sequence is in its new version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moved {
    /// Found unchanged at this index.
    Same(usize),
    /// Edited: the element at this index stands where it was.
    Edited(usize),
    /// Removed: this index holds what followed it (or is the last one).
    Removed(usize),
}

/// Finds `old[ix]` in `new`, the same sequence after an edit. `None` when
/// `new` is empty or `ix` is out of range.
///
/// Linear in the length of the sequences, so it's fine for huge documents:
/// the common prefix and suffix are unchanged, and only the part between
/// them is hashed. There, elements that occur exactly once in both versions
/// are anchors. `old[ix]` itself, if it's one, is found directly. Otherwise
/// the nearest anchors before and after it bound the window it must be in:
/// a copy of it there, nearest to where it would be, wins over copies
/// elsewhere (repeated headings like `### Added`); with no copy it was
/// edited in place, or removed when the window is empty.
pub fn find_moved<T: Hash + Eq>(old: &[T], new: &[T], ix: usize) -> Option<Moved> {
    if new.is_empty() || ix >= old.len() {
        return None;
    }
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    if ix < prefix {
        return Some(Moved::Same(ix));
    }
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (old_end, new_end) = (old.len() - suffix, new.len() - suffix);
    if ix >= old_end {
        return Some(Moved::Same(ix - old_end + new_end));
    }
    let moved = match match_changed(&old[prefix..old_end], &new[prefix..new_end], ix - prefix) {
        Moved::Same(j) => Moved::Same(prefix + j),
        Moved::Edited(j) => Moved::Edited(prefix + j),
        Moved::Removed(j) => Moved::Removed((prefix + j).min(new.len() - 1)),
    };
    Some(moved)
}

/// [`find_moved`] between the changed parts of two sequences, which have no
/// common prefix or suffix. `new` may be empty.
fn match_changed<T: Hash + Eq>(old: &[T], new: &[T], ix: usize) -> Moved {
    let key = |t: &T| {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    };
    let old_keys: Vec<u64> = old.iter().map(key).collect();
    let new_keys: Vec<u64> = new.iter().map(key).collect();
    // Per hash: occurrences in `old`, in `new`, and the last index in `new`.
    let mut seen: HashMap<u64, (u32, u32, usize)> = HashMap::with_capacity(old.len() + new.len());
    for &k in &old_keys {
        seen.entry(k).or_default().0 += 1;
    }
    for (j, &k) in new_keys.iter().enumerate() {
        let e = seen.entry(k).or_default();
        e.1 += 1;
        e.2 = j;
    }
    // Where `old[i]` is in `new`, if it occurs once in each (and isn't
    // just a hash collision).
    let anchor = |i: usize| {
        let &(in_old, in_new, j) = seen.get(&old_keys[i])?;
        (in_old == 1 && in_new == 1 && old[i] == new[j]).then_some(j)
    };
    if let Some(j) = anchor(ix) {
        return Moved::Same(j);
    }
    let before = (0..ix).rev().find_map(|i| Some((i, anchor(i)?)));
    let after = (ix + 1..old.len()).find_map(|i| Some((i, anchor(i)?)));
    let lo = before.map_or(0, |(_, j)| j + 1);
    // A moved block can put the next anchor before the previous one: then
    // only the one before counts.
    let hi = after
        .map(|(_, j)| j)
        .filter(|&j| j >= lo)
        .unwrap_or(new.len());
    if lo == hi {
        return Moved::Removed(hi);
    }
    let guess = match (before, after) {
        (Some((i, j)), _) => j + (ix - i),
        (None, Some((i, j))) => j.saturating_sub(i - ix),
        (None, None) => ix,
    }
    .clamp(lo, hi - 1);
    let copy = (lo..hi)
        .filter(|&j| new_keys[j] == old_keys[ix] && new[j] == old[ix])
        .min_by_key(|&j| j.abs_diff(guess));
    match copy {
        Some(j) => Moved::Same(j),
        None => Moved::Edited(guess),
    }
}

/// Where row `sub` of a top-level block that was edited (`old` became
/// `new`) went, for blocks split into rows by their parts: table body rows,
/// list items, and the child blocks of a quote (a quote of one block, which
/// is one row, counts as that block). `None` for other blocks, or when the
/// block changed kind.
fn sub_row(old: &Block, new: &Block, sub: usize) -> Option<Moved> {
    // `parts` of a block starting at row `head` (after a header row).
    let shift = |m: Moved, head: usize| match m {
        Moved::Same(j) => Moved::Same(j + head),
        Moved::Edited(j) => Moved::Edited(j + head),
        Moved::Removed(j) => Moved::Removed(j + head),
    };
    match (old, new) {
        (Block::Table { rows: old_rows, .. }, Block::Table { rows: new_rows, .. }) => {
            if sub == 0 {
                return Some(Moved::Edited(0));
            }
            Some(shift(find_moved(old_rows, new_rows, sub - 1)?, 1))
        }
        (
            Block::List {
                items: old_items, ..
            },
            Block::List {
                items: new_items, ..
            },
        ) => find_moved(old_items, new_items, sub),
        (
            Block::Quote {
                callout: old_callout,
                blocks: old_children,
            },
            Block::Quote {
                callout: new_callout,
                blocks: new_children,
            },
        ) => {
            let (old_head, new_head) = (
                quote_head(*old_callout, old_children),
                quote_head(*new_callout, new_children),
            );
            if sub < old_head {
                return Some(Moved::Edited(0));
            }
            Some(shift(
                find_moved(old_children, new_children, sub - old_head)?,
                new_head,
            ))
        }
        _ => None,
    }
}

/// Rows before the first child block of a quote: its callout title, which
/// only has a row of its own when the quote is split (see `from_blocks`).
fn quote_head(callout: Option<Callout>, children: &[Block]) -> usize {
    usize::from(callout.is_some() && children.len() > 1)
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

    /// A parsed document and its rows.
    struct Parsed {
        doc: Document,
        rows: Rows,
    }

    impl Parsed {
        fn new(src: &str) -> Self {
            let doc = markdown::parse(src);
            let rows = Rows::from_document(&doc);
            Self { doc, rows }
        }

        /// `old_row` of `old`, remapped into `self`.
        fn remap(&self, old: &Parsed, old_row: usize) -> RemappedRow {
            self.rows
                .remap_row(&self.doc.blocks, &old.rows, &old.doc.blocks, old_row)
        }
    }

    #[test]
    fn block_and_offset_and_remap_preserve_block_across_table_edits() {
        let make_doc = |n: usize| {
            let mut s = String::from("| ID |\n|---|\n");
            for i in 0..n {
                s.push_str(&format!("| {i} |\n"));
            }
            s.push_str("\n## After Table\n\nTrailing paragraph.\n");
            Parsed::new(&s)
        };

        let small = make_doc(5); // block 0: rows 0..=5, block 1 (heading): row 6, block 2: row 7
        let large = make_doc(105); // block 0: rows 0..=105, block 1 (heading): row 106, block 2: row 107

        // Every Row variant reports its owning block and offset.
        assert_eq!(small.rows.block_and_offset(0), Some((0, 0)));
        assert_eq!(small.rows.block_and_offset(5), Some((0, 5)));
        assert_eq!(small.rows.block_and_offset(6), Some((1, 0)));
        assert_eq!(small.rows.block_and_offset(7), Some((2, 0)));
        assert_eq!(small.rows.block_and_offset(8), None);

        // Growing the preceding table from 5 to 105 rows keeps the scroll
        // position on `## After Table` (block 1 -> row 106, not row 6) and
        // preserves the pixel offset inside that heading.
        assert_eq!(
            large.remap(&small, 6),
            RemappedRow {
                item_ix: 106,
                reset_offset: false
            }
        );
        assert_eq!(
            large.remap(&small, 7),
            RemappedRow {
                item_ix: 107,
                reset_offset: false
            }
        );
        // Shrinking the table back returns to row 6 without resetting offset.
        assert_eq!(
            small.remap(&large, 106),
            RemappedRow {
                item_ix: 6,
                reset_offset: false
            }
        );
        // Staying inside the table preserves the row offset, and shrinking
        // clamps to the table's last row (resetting pixel offset) without
        // spilling into `## After Table`.
        assert_eq!(
            large.remap(&small, 3),
            RemappedRow {
                item_ix: 3,
                reset_offset: false
            }
        );
        assert_eq!(
            small.remap(&large, 50),
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
            Parsed::new(&s)
        };
        let pref_large = with_prefix(20); // block 0: row 0, block 1: rows 1..=21, block 2: row 22
        let pref_small = with_prefix(3); // block 0: row 0, block 1: rows 1..=4, block 2: row 5
        assert_eq!(
            pref_small.remap(&pref_large, 15),
            RemappedRow {
                item_ix: 4,
                reset_offset: true
            }
        );
        assert_eq!(
            pref_small.remap(&pref_large, 22),
            RemappedRow {
                item_ix: 5,
                reset_offset: false
            }
        );
        // Top of document stays at 0; out-of-range (e.g. Bottom) clamps to end.
        assert_eq!(
            large.remap(&small, 0),
            RemappedRow {
                item_ix: 0,
                reset_offset: false
            }
        );
        assert_eq!(
            large.remap(&small, small.rows.len()),
            RemappedRow {
                item_ix: large.rows.len() - 1,
                reset_offset: true
            }
        );
        // Reloading into an empty doc or shorter block list clamps safely.
        let empty = Parsed::new("");
        assert_eq!(
            empty.remap(&small, 6),
            RemappedRow {
                item_ix: 0,
                reset_offset: true
            }
        );
        assert_eq!(
            small.remap(&empty, 0),
            RemappedRow {
                item_ix: 0,
                reset_offset: true
            }
        );
        // The trailing paragraph became the only one: edited in place, so
        // the offset stays (the list clamps it if the row got shorter).
        let one_para = Parsed::new("Only one block\n");
        assert_eq!(
            one_para.remap(&small, 7),
            RemappedRow {
                item_ix: 0,
                reset_offset: false
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
            Parsed::new(&s)
        };
        let code_150 = code_doc(150);
        let code_200 = code_doc(200);
        let code_70 = code_doc(70);
        let code_1 = code_doc(1);
        assert_eq!(
            code_1.remap(&code_150, 1),
            RemappedRow {
                item_ix: 0,
                reset_offset: true
            }
        );
        assert_eq!(
            code_1.remap(&code_150, 0),
            RemappedRow {
                item_ix: 0,
                reset_offset: true
            }
        );
        assert_eq!(
            code_70.remap(&code_150, 1),
            RemappedRow {
                item_ix: 1,
                reset_offset: true
            }
        );
        assert_eq!(
            code_200.remap(&code_150, 1),
            RemappedRow {
                item_ix: 1,
                reset_offset: false
            }
        );
    }

    fn moved(old: &[&str], new: &[&str], ix: usize) -> Option<Moved> {
        find_moved(old, new, ix)
    }

    #[test]
    fn find_moved_follows_insertions_and_deletions_around_it() {
        let old = ["a", "b", "c", "d", "e"];
        // Unchanged prefix and suffix.
        assert_eq!(
            moved(&old, &["a", "X", "b", "c", "d", "e"], 0),
            Some(Moved::Same(0))
        );
        assert_eq!(
            moved(&old, &["a", "X", "b", "c", "d", "e"], 3),
            Some(Moved::Same(4))
        );
        assert_eq!(moved(&old, &["a", "c", "d", "e"], 3), Some(Moved::Same(2)));
        assert_eq!(moved(&old, &old, 4), Some(Moved::Same(4)));
        // Edits both above and below: found as an anchor in the middle.
        let new = ["X", "a", "b", "c", "Y", "d", "e!"];
        assert_eq!(moved(&old, &new, 2), Some(Moved::Same(3)));
        assert_eq!(moved(&old, &new, 3), Some(Moved::Same(5)));
        assert_eq!(moved(&old, &new, 4), Some(Moved::Edited(6)));
        // Nothing to find in.
        assert_eq!(moved(&old, &[], 0), None);
        assert_eq!(moved(&old, &old, 5), None);
        assert_eq!(moved(&[], &old, 0), None);
    }

    #[test]
    fn find_moved_picks_the_copy_of_a_repeated_element_nearest_its_neighbors() {
        // A changelog: every release has an `Added` heading. The title and
        // the footer changed too, so everything in between is compared.
        let old = ["T", "1.1", "Added", "x", "1.0", "Added", "y", "end"];
        let new = [
            "T2", "1.2", "Added", "z", "1.1", "Added", "x", "1.0", "Added", "y", "end2",
        ];
        assert_eq!(moved(&old, &new, 2), Some(Moved::Same(5)));
        assert_eq!(moved(&old, &new, 5), Some(Moved::Same(8)));
        // Only the anchor after it: counted back from there.
        let old = ["T", "Added", "x", "end"];
        let new = ["T2", "Added", "w", "Added", "x", "end2"];
        assert_eq!(moved(&old, &new, 1), Some(Moved::Same(3)));
        // No anchor at all: the copy nearest to the old index.
        let old = ["T", "-", "-", "-", "end"];
        let new = ["T2", "-", "-", "-", "-", "end2"];
        assert_eq!(moved(&old, &new, 2), Some(Moved::Same(2)));
        assert_eq!(moved(&old, &new, 3), Some(Moved::Same(3)));
    }

    #[test]
    fn find_moved_keeps_an_edited_element_between_its_neighbors() {
        let old = ["T", "x", "para", "y", "end"];
        let new = ["T2", "x", "para, edited", "y", "end2"];
        assert_eq!(moved(&old, &new, 2), Some(Moved::Edited(2)));
        // Something was also inserted before it: still right after `x`.
        let new = ["T2", "N", "x", "para, edited", "y", "end2"];
        assert_eq!(moved(&old, &new, 2), Some(Moved::Edited(3)));
        // Its neighbors are copies too: where it was, counted from the
        // anchor before it.
        let old = ["T", "x", "-", "p", "-", "y", "end"];
        let new = ["T2", "x", "-", "p2", "-", "y", "end2"];
        assert_eq!(moved(&old, &new, 3), Some(Moved::Edited(3)));
        // One edit: just the element itself differs.
        let old = ["a", "b", "c"];
        assert_eq!(moved(&old, &["a", "B", "c"], 1), Some(Moved::Edited(1)));
        assert_eq!(
            moved(&old, &["a", "B", "B2", "c"], 1),
            Some(Moved::Edited(1))
        );
        // Without anchors, the guess stays inside the new middle.
        assert_eq!(moved(&["a", "b", "c"], &["z"], 2), Some(Moved::Edited(0)));
    }

    #[test]
    fn find_moved_reports_removed_elements() {
        let old = ["a", "b", "c", "d"];
        assert_eq!(moved(&old, &["a", "c", "d"], 1), Some(Moved::Removed(1)));
        assert_eq!(moved(&old, &["a", "d"], 2), Some(Moved::Removed(1)));
        // At the end: the last element left.
        assert_eq!(moved(&old, &["a", "b"], 3), Some(Moved::Removed(1)));
        assert_eq!(moved(&old, &["a", "b"], 2), Some(Moved::Removed(1)));
        // Between two anchors with nothing left in between.
        let old = ["T", "x", "gone", "y", "end"];
        let new = ["T2", "x", "y", "end2"];
        assert_eq!(moved(&old, &new, 2), Some(Moved::Removed(2)));
    }

    #[test]
    fn find_moved_compares_elements_not_just_hashes() {
        /// Every value hashes the same.
        #[derive(PartialEq, Eq)]
        struct Collide(u8);
        impl Hash for Collide {
            fn hash<H: Hasher>(&self, _: &mut H) {}
        }
        let c = |v: &[u8]| v.iter().map(|&b| Collide(b)).collect::<Vec<_>>();
        let old = c(b"TabcE");
        // No anchors (all one hash): the copy nearest the old index.
        assert_eq!(find_moved(&old, &c(b"UabcF"), 2), Some(Moved::Same(2)));
        assert_eq!(find_moved(&old, &c(b"UaXbcF"), 2), Some(Moved::Same(3)));
        assert_eq!(find_moved(&old, &c(b"UaZcF"), 2), Some(Moved::Edited(2)));
        // One element per hash, but not equal: not an anchor.
        let old = c(b"Tb");
        assert_eq!(find_moved(&old, &c(b"Uc"), 1), Some(Moved::Edited(1)));
    }

    #[test]
    fn find_moved_stays_in_range_when_blocks_move() {
        // `A` and `B` swapped: the anchors cross.
        let old = ["P", "A", "Q", "B", "R"];
        let new = ["P2", "B", "Q2", "A", "R2"];
        for ix in 0..old.len() {
            let m = moved(&old, &new, ix).unwrap();
            let (Moved::Same(j) | Moved::Edited(j) | Moved::Removed(j)) = m;
            assert!(j < new.len(), "{ix}: {m:?}");
        }
        assert_eq!(moved(&old, &new, 1), Some(Moved::Same(3)));
        assert_eq!(moved(&old, &new, 3), Some(Moved::Same(1)));
        assert_eq!(moved(&old, &new, 2), Some(Moved::Edited(4)));
    }

    /// Issue #24: the reader stays on the section they were reading when
    /// blocks are inserted or removed above it, or the block is edited.
    #[test]
    fn remap_follows_the_block_across_edits_above_it() {
        let base =
            "# Title\n\n## One\n\nFirst.\n\n## Two\n\nSecond.\n\n---\n\n## Three\n\nThird.\n";
        let old = Parsed::new(base);
        // Rows: 0 title, 1 `## One`, 2 First, 3 `## Two`, 4 Second, 5 rule,
        // 6 `## Three`, 7 Third.
        let at = |new: &Parsed, row| new.remap(&old, row);
        let same = |item_ix| RemappedRow {
            item_ix,
            reset_offset: false,
        };
        // A heading and a paragraph inserted above `## Two`.
        let inserted = Parsed::new(&base.replace("## One", "## Zero\n\nNew.\n\n## One"));
        assert_eq!(at(&inserted, 3), same(5));
        assert_eq!(at(&inserted, 7), same(9));
        // A block above it deleted.
        let deleted = Parsed::new(&base.replace("First.\n\n", ""));
        assert_eq!(at(&deleted, 3), same(2));
        assert_eq!(at(&deleted, 6), same(5));
        // Inserted above and edited below: `Second.` is matched by content.
        let both = Parsed::new(
            &base
                .replace("# Title", "# Title\n\nIntro.")
                .replace("Third.", "Third, edited."),
        );
        assert_eq!(at(&both, 4), same(5));
        // The paragraph at the top edited, plus an edit elsewhere: the
        // viewport stays on it, keeping the offset.
        let edited = Parsed::new(
            &base
                .replace("Second.", "Second, with a typo fixed.")
                .replace("# Title", "# Title\n\nIntro."),
        );
        assert_eq!(at(&edited, 4), same(5));
        // The block at the top removed: the next one, from its top.
        let gone = Parsed::new(&base.replace("Second.\n\n", ""));
        assert_eq!(
            at(&gone, 4),
            RemappedRow {
                item_ix: 4,
                reset_offset: true
            }
        );
    }

    #[test]
    fn remap_follows_rows_inside_edited_tables_lists_and_quotes() {
        let list = |items: &[&str]| {
            let body: String = items.iter().map(|i| format!("- {i}\n")).collect();
            Parsed::new(&format!("# T\n\n{body}\n## End\n"))
        };
        let same = |item_ix| RemappedRow {
            item_ix,
            reset_offset: false,
        };
        // Rows: 0 title, 1.. items.
        let old = list(&["a", "b", "c"]);
        assert_eq!(list(&["new", "a", "b", "c"]).remap(&old, 3), same(4));
        assert_eq!(list(&["b", "c"]).remap(&old, 3), same(2));
        assert_eq!(list(&["a", "B", "c"]).remap(&old, 2), same(2));
        assert_eq!(
            list(&["a", "c"]).remap(&old, 2),
            RemappedRow {
                item_ix: 2,
                reset_offset: true
            }
        );

        let table = |rows: &[&str]| {
            let body: String = rows.iter().map(|r| format!("| {r} |\n")).collect();
            Parsed::new(&format!("| H |\n|---|\n{body}\nAfter\n"))
        };
        // Rows: 0 header, 1.. body rows.
        let old = table(&["a", "b", "c"]);
        assert_eq!(table(&["new", "a", "b", "c"]).remap(&old, 3), same(4));
        assert_eq!(table(&["new", "a", "b", "c"]).remap(&old, 0), same(0));
        assert_eq!(table(&["b", "c"]).remap(&old, 2), same(1));
        // Edited: its row, keeping the offset.
        assert_eq!(table(&["a", "B", "c"]).remap(&old, 2), same(2));
        // Removed: the row that followed it, from its top.
        assert_eq!(
            table(&["a", "c"]).remap(&old, 2),
            RemappedRow {
                item_ix: 2,
                reset_offset: true
            }
        );

        let quote = |callout: &str, children: &[&str]| {
            let body = children
                .iter()
                .map(|c| format!("> {c}\n"))
                .collect::<Vec<_>>()
                .join(">\n");
            Parsed::new(&format!("{callout}{body}\nAfter\n"))
        };
        // Rows: 0 callout header, 1.. children.
        let old = quote("> [!NOTE]\n", &["a", "b", "c"]);
        assert_eq!(
            quote("> [!NOTE]\n", &["new", "a", "b", "c"]).remap(&old, 1),
            same(2)
        );
        assert_eq!(
            quote("> [!NOTE]\n", &["new", "a", "b", "c"]).remap(&old, 2),
            same(3)
        );
        assert_eq!(
            quote("> [!TIP]\n", &["a", "b", "c", "d"]).remap(&old, 0),
            same(0)
        );
        // The callout went away: children start at row 0.
        assert_eq!(quote("", &["new", "a", "b", "c"]).remap(&old, 3), same(3));
        // And back.
        let plain = quote("", &["a", "b"]);
        assert_eq!(
            quote("> [!NOTE]\n", &["a", "b", "c"]).remap(&plain, 1),
            same(2)
        );
        // A quote that became one block isn't split any more: its one row.
        assert_eq!(
            quote("", &["only"]).remap(&old, 2),
            RemappedRow {
                item_ix: 0,
                reset_offset: true
            }
        );
        // A one-block callout (one row, no title row of its own) counts as
        // its block: split by a block added above, the row is that block's.
        let single = quote("> [!NOTE]\n", &["a"]);
        assert_eq!(
            quote("> [!NOTE]\n", &["new", "a"]).remap(&single, 0),
            RemappedRow {
                item_ix: 2,
                reset_offset: true
            }
        );
        // Edited, still one block: its row, keeping the offset.
        assert_eq!(
            quote("> [!NOTE]\n", &["a, edited"]).remap(&single, 0),
            same(0)
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
