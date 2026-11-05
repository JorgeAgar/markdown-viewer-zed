# Vendored specification examples

Inputs for `tests/spec.rs`. Unmodified copies of:

| File | Source |
| --- | --- |
| `commonmark-0.31.2.json` | <https://spec.commonmark.org/0.31.2/spec.json> (the 652 examples of the CommonMark Spec 0.31.2) |
| `gfm-spec.txt` | <https://github.com/github/cmark-gfm/blob/27d942c8b0a62d192f616e5bf3578f4b6a89e180/test/spec.txt> (GitHub Flavored Markdown Spec 0.29-gfm, 672 examples) |

Both specs are by John MacFarlane (GFM additions by GitHub) and licensed
under the [Creative Commons Attribution-ShareAlike 4.0 license
(CC-BY-SA 4.0)](https://creativecommons.org/licenses/by-sa/4.0/). They are
test data only; nothing from them is compiled into mdview.

To update, download the files again from the URLs above (bump the version
or commit in this table) and run `cargo test -p mdview-core --test spec`.
