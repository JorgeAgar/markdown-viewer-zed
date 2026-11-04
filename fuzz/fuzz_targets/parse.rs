//! `markdown::parse` on arbitrary UTF-8, checking every invariant of the
//! result (spans, links, tables, slugs, code highlights...).
#![no_main]

// The invariants the property and spec tests use.
#[path = "../../crates/mdview-core/tests/common/invariants.rs"]
mod invariants;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(src) = std::str::from_utf8(data) {
        invariants::parse_and_check(src);
    }
});
