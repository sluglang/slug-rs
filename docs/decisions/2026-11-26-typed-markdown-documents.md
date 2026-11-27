# Typed Markdown documents

## Status

Accepted (2026-11-26). Extends
[safe Markdown rendering](2026-11-16-safe-markdown-rendering.md).

## Context

The SSG application needs source-aware link rewriting, heading IDs, tables of
contents, code inspection, and plain text. Its milestone 4 cannot use the
renderer-only API. Internal blocks still contain unparsed inline strings;
exporting their maps would expose parser bookkeeping and require consumers
to reproduce reference resolution.

## Decision

Expose a fully parsed, immutable, constructible `Node` payload enum and
`Document`, `Position`, and `Span` structs through `slug.text.markdown`.
[Markdown documents](../reference/markdown-documents.md) owns the public API,
node fields, traversal, provenance, errors, and budgets. Parsing records
original-source provenance and resolves reference destinations. Transformations
replace one node post-order. Applications assign heading IDs and resolve routes.

Render typed documents through the existing safe renderer, with structural
validation and independent tree/output budgets. Retain `toHtml`'s fast existing
path and output; prove parity against the CommonMark/GFM corpus. Do not expose
raw HTML injection or general renderer callbacks.

## Consequences

Applications can use ordinary matching and persistent copies to transform
documents. Parser maps remain private. Source tracking must survive container
prefix removal, entity decoding, references, tables, and newline normalization.
Tree category and range checks remain necessary even with nominal node types.
Highlighting and heading slug policies are separate work.

## Migration

None. Existing `toHtml` callers keep their API and defaults.
