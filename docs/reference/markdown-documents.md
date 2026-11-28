# Markdown documents

`slug.text.markdown` provides an immutable, constructible typed tree. Parsing
resolves all inline syntax and reference destinations; rendering never reparses
node text. `toHtml(source, options)` retains its existing behavior and limits.

```slug
val md = import("slug.text.markdown")
val document = md.parse("# Guide\n\n[Next](compatibility.md)")
val updated = md.transform(document, fn(node:md.Node):md.Node {
    match node {
        link: md.Node.Link => link + {url: "/next/"}
        heading: md.Node.Heading => heading + {id: "guide"}
        _ => node
    }
})
val html = md.render(updated)
val text = md.toText(updated)
```

## Types

`Position {offset, line, column}` uses zero-based Unicode scalar offsets into
the original Slug source string and one-based lines and columns. A tab counts
as one scalar column. CRLF and bare CR each end one line. `Span {start, end}`
has an exclusive end. Normalization of newlines and NUL does not change these
coordinates. Spans enclose source syntax, not just decoded text; containers
enclose their content, including intervening container markers. They describe
provenance, not a source-edit or formatting-preservation contract.

`Position`, `Span`, and `Document` export structural record aliases and struct
constructors of the same names. `Document {children:list<Node>, span:Span|nil = nil}`
contains block nodes; when using a map literal instead of the constructor, supply
the `span` field explicitly. The defaults below belong to the constructors.
Every payload variant of the `Node` enum has `span:Span|nil = nil`:

| Variant | Other fields |
| --- | --- |
| `Text`, `CodeSpan`, `RawHtml` | `text:str` |
| `SoftBreak`, `HardBreak`, `ThematicBreak` | None |
| `Emphasis`, `Strong`, `Strikethrough`, `Paragraph`, `BlockQuote`, `Item` | `children:list<Node>` |
| `Link`, `Image` | `url:str`, `title:str = ""`, `children:list<Node>`, `definition:Span|nil = nil` |
| `Heading` | `level:num`, `children:list<Node>`, `id:str|nil = nil` |
| `CodeBlock` | `text:str`, `info:str = ""` |
| `List` | `children:list<Node>`, `ordered:bool = false`, `start:num = 1`, `tight:bool = false` |
| `Table` | `aligns:list<str>`, `head:list<list<Node>>`, `rows:list<list<list<Node>>>` |

Paragraphs, headings, inline formatting, links, images, and table cells contain
inline nodes. Block quotes and items contain blocks. Lists contain only items;
items occur only in lists. Table rows have the same width as the alignments;
each alignment is `""`, `"left"`, `"center"`, or `"right"`. Heading levels are
integers from 1 through 6; ordered-list starts are integers from 0 through
999999999. Heading IDs are nonempty strings without ASCII whitespace or
control characters. Duplicate IDs and ID generation are application policies.

Parsed reference links retain their occurrence in `span` and the supplying
definition in `definition`. Direct links and autolinks have no definition
span. Unresolved reference syntax remains ordinary Markdown text. Raw HTML
nodes always render as escaped text. Constructed nodes may omit provenance;
A field update with `+` retains it unless the application replaces or clears it.

## Operations and limits

- `parse(source:str, options:map = {}):Document` parses without I/O.
- `transform(document:Document, rewrite:fn<Node, Node>, options:map = {}):Document`
  visits nodes in source/tree order, children before parents. The document
  wrapper is not visited. Each original node is replaced by exactly one node;
  replacements are validated but not revisited. The input is unchanged and
  callback errors propagate.
- `render(document:Document, options:map = {}):str` validates and
  renders the tree under the existing escaping and link/image scheme policy.
  It also supports the existing `links`, `images`, and `rel` options.
- `toText(document:Document, options:map = {}):str` extracts decoded
  text, code, literal raw HTML, and image descriptions. Inline breaks and block
  boundaries become newlines, table cells are separated by tabs, and rows by
  newlines. Code-block text retains its terminating newline. No HTML escaping,
  slug generation, whitespace collapsing, or URL text is added.

All four operations accept positive-integer `maxBytes` (default 65536),
`maxDepth` (32), and `maxNodes` (65536), within Slug's signed integer range.
Parsing checks original UTF-8 source bytes. Tree operations check aggregate
UTF-8 text fields (including titles,
URLs, IDs, code info, and alignments). Block containers and inline containers
have separate depth budgets, as in `toHtml`. Nodes, table rows, and table cells
count toward the node/work budget. A transform checks both its input and its
result. Parsing enforces the node budget while constructing the public tree;
its preceding normalization and internal parsing are bounded by source bytes
and nesting. These limits bound library traversal, not arbitrary callback work.

Rendering additionally accepts `maxOutputBytes` (default 1048576), enforced
while emitting HTML. Text extraction also bounds output by `maxBytes`.
Bad options throw `markdown.options`; exceeded budgets throw `markdown.limit`
with `data.limit` and `data.max`. Invalid tree structure, field ranges, or
provenance throws `markdown.document`. Nominal and field annotation violations
may instead be rejected by Slug's ordinary checked type errors. No arbitrary
HTML output callback or raw-HTML enabling option is provided.

The parser helper modules remain implementation details. Their maps, reference
tables, and bookkeeping are not the public document model.
