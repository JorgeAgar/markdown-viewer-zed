Inline [link](https://example.com) and [with title](https://example.com "Title").

Reference [full][ref], [collapsed][], and [shortcut].

[ref]: https://example.com/ref
[collapsed]: https://example.com/collapsed
[shortcut]: <https://example.com/short cut>

Autolinks <https://example.com/auto> and <mailto:me@example.com> and <me@example.com>.

Bare URLs https://example.com/bare and www.example.com are not links.

Styled: [**bold** and *italic* text](u1), **[bold link](u2)**, *[italic](u3) after*.

Crossing: **bold [link** text](u4) and [link **bold](u5) text**.

Code in link: [`code` text](u6). Empty text: [](u7). Anchor: [section](#heading-slug).

Relative: [doc](other.md), [doc with anchor](other.md#part), [dir](../dir/file.txt).

[Missing reference][nope] stays text, and [brackets] too.

Escaped \[not a link\](u8) and [link with \] bracket](u9).

[a](u10)[b](u11) adjacent, and [same](u12) [same](u12) twice.

Percent-encoded [space](my%20file.md) and [unicode](ü.md) and [query](https://x.y/?a=1&b=2).
