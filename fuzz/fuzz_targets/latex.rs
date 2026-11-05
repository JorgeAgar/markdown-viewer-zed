//! `latex::to_unicode` on arbitrary UTF-8, and the same text as inline math
//! in a paragraph.
#![no_main]

#[path = "../../crates/mdview-core/tests/common/invariants.rs"]
mod invariants;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(tex) = std::str::from_utf8(data) {
        invariants::to_unicode_and_check(tex);
        if !tex.contains(['$', '\n', '`']) {
            invariants::parse_and_check(&format!("a ${tex}$ b"));
        }
    }
});
