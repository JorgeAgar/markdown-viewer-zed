//! Every example of the CommonMark 0.31.2 and GFM specs (`tests/data/`)
//! goes through `parse` and must satisfy the model's invariants. The model
//! is lossy (no HTML), so instead of comparing HTML, a structural check
//! counts headings, code blocks, quotes, list items, tables and rules in
//! the spec's expected HTML and in the document, for the examples where
//! mdview's extensions (math, footnotes, callouts, front matter) and HTML
//! blocks can't change the structure.

mod common;

use common::invariants::parse_and_check;
use mdview_core::markdown::Block;

struct Example {
    spec: &'static str,
    number: usize,
    section: String,
    markdown: String,
    html: String,
}

fn commonmark() -> Vec<Example> {
    let json: serde_json::Value =
        serde_json::from_str(include_str!("data/commonmark-0.31.2.json")).unwrap();
    json.as_array()
        .unwrap()
        .iter()
        .map(|e| Example {
            spec: "CommonMark",
            number: e["example"].as_u64().unwrap() as usize,
            section: e["section"].as_str().unwrap().to_string(),
            markdown: e["markdown"].as_str().unwrap().to_string(),
            html: e["html"].as_str().unwrap().to_string(),
        })
        .collect()
}

/// The examples of a spec.txt: between a line of 32 backticks + " example"
/// and a line of 32 backticks, markdown and HTML separated by a "." line,
/// with tabs written as "→".
fn gfm() -> Vec<Example> {
    let fence = "`".repeat(32);
    let mut out = Vec::new();
    let mut section = String::new();
    let mut lines = include_str!("data/gfm-spec.txt").lines();
    while let Some(line) = lines.next() {
        if let Some(h) = line.strip_prefix('#') {
            section = h.trim_start_matches('#').trim().to_string();
        }
        if !(line.starts_with(&fence) && line.contains("example")) {
            continue;
        }
        let (mut md, mut html, mut in_html) = (String::new(), String::new(), false);
        for l in lines.by_ref() {
            if l == fence {
                break;
            }
            if l == "." && !in_html {
                in_html = true;
                continue;
            }
            let target = if in_html { &mut html } else { &mut md };
            target.push_str(&l.replace('→', "\t"));
            target.push('\n');
        }
        out.push(Example {
            spec: "GFM",
            number: out.len() + 1,
            section: section.clone(),
            markdown: md,
            html,
        });
    }
    out
}

#[test]
fn spec_files_are_complete() {
    assert_eq!(commonmark().len(), 652);
    assert_eq!(gfm().len(), 672);
}

#[test]
fn every_example_satisfies_the_invariants() {
    for e in commonmark().iter().chain(&gfm()) {
        let r = std::panic::catch_unwind(|| parse_and_check(&e.markdown));
        assert!(
            r.is_ok(),
            "{} example {} ({}): {:?}",
            e.spec,
            e.number,
            e.section,
            e.markdown
        );
    }
}

#[derive(Debug, Default, PartialEq)]
struct Counts {
    headings: usize,
    code: usize,
    quotes: usize,
    items: usize,
    tables: usize,
    rules: usize,
}

fn count_html(html: &str) -> Counts {
    let n = |pat: &str| html.matches(pat).count();
    Counts {
        headings: (1..=6).map(|i| n(&format!("<h{i}>"))).sum(),
        code: n("<pre><code"),
        quotes: n("<blockquote>"),
        items: n("<li>") + n("<li class"),
        tables: n("<table>"),
        rules: n("<hr />"),
    }
}

fn count_doc(blocks: &[Block], c: &mut Counts) {
    for b in blocks {
        match b {
            Block::Heading { .. } => c.headings += 1,
            Block::Code { .. } | Block::Math(_) | Block::Mermaid(_) => c.code += 1,
            Block::Quote { blocks, .. } => {
                c.quotes += 1;
                count_doc(blocks, c);
            }
            Block::List { items, .. } => {
                c.items += items.len();
                for item in items {
                    count_doc(&item.blocks, c);
                }
            }
            Block::Table { .. } => c.tables += 1,
            Block::Rule => c.rules += 1,
            Block::Footnote { blocks, .. } => count_doc(blocks, c),
            Block::Paragraph(_) | Block::Images(_) => {}
        }
    }
}

/// Examples where mdview's structure legitimately differs from the spec's
/// HTML, with the reason. Everything else must match exactly.
const STRUCTURE_EXCEPTIONS: &[(&str, usize, &str)] = &[];

#[test]
fn structure_matches_the_spec_html() {
    let mut checked = 0;
    let mut failures = Vec::new();
    for e in commonmark().iter().chain(&gfm()) {
        let md = &e.markdown;
        // mdview's extensions and skipped HTML change the structure.
        let skip = e.section.contains("HTML")
            || md.contains('<')
            || md.contains('$')
            || md.contains("[^")
            || md.contains("[!")
            || md.starts_with("---")
            || md.starts_with("+++")
            || md.starts_with('\u{feff}')
            || (e.spec == "CommonMark"
                && (md.contains('~')
                    || md.contains('|')
                    || md.contains("[ ]")
                    || md.contains("[x]")));
        if skip {
            continue;
        }
        checked += 1;
        let mut got = Counts::default();
        count_doc(&parse_and_check(md).blocks, &mut got);
        let want = count_html(&e.html);
        let excepted = STRUCTURE_EXCEPTIONS
            .iter()
            .any(|(s, n, _)| *s == e.spec && *n == e.number);
        if (got != want) != excepted {
            failures.push(format!(
                "{} example {} ({}): {md:?}\n  spec: {want:?}\n  ours: {got:?}{}",
                e.spec,
                e.number,
                e.section,
                if excepted {
                    " (listed as an exception, but matches)"
                } else {
                    ""
                }
            ));
        }
    }
    assert!(checked > 900, "only {checked} examples checked");
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
