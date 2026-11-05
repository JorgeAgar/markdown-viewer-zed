//! mdview's GUI-free core: everything that turns Markdown text into the block
//! model the viewer renders. Nothing here may depend on GPUI; see
//! `docs/architecture.md`.

pub mod highlight;
pub mod latex;
pub mod markdown;
