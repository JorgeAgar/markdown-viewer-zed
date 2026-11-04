//! `highlight::highlight` for every language on arbitrary UTF-8.
#![no_main]

#[path = "../../crates/mdview-core/tests/common/invariants.rs"]
mod invariants;
#[path = "../../crates/mdview-core/tests/common/langs.rs"]
mod langs;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(code) = std::str::from_utf8(data) {
        for lang in langs::all_langs() {
            invariants::highlight_and_check(lang, code);
        }
    }
});
