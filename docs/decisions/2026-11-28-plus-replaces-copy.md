# `+` replaces `copy`

## Status

Accepted

## Supersedes

[Preserve schema identity when copying structs](2026-08-22-struct-copy-semantics-superseded.md),
and revises [Merging into a record type](2026-11-24-record-merge.md).

## Context

Since the [record merge](2026-11-24-record-merge.md), `struct + {k: v}` made the
same copy as `value copy { k: v }` at run time, so the language had two
spellings of one update. They were not equal before the checker: on a value it
knew to be a `struct<S>`, `+` was rejected outright, lost `copy`'s field checks
and messages, and, building its map first, ran about 1.5 times slower. `copy`
also grouped differently: it is a postfix form, binding as tightly as `.field`,
where `+` is an ordinary binary operator.

## Decision

`value + { field: v, ... }` is the one way to update fields, and `copy` is
removed.

- **On a known `struct<S>`, `+` checks what `copy` checked.** Each constant
  key must be a field of `S`, its value of the field's type (with a generic
  schema's arguments), and no key may repeat; the result is a `struct<S>`.
- **Keys known only when it runs are allowed,** a computed key,
  `s + {[k]: v}`, or a map built at run time, `s + changes`, on a struct or a
  record type, if the keys may be strings. When it runs, a struct fails on a
  key its schema lacks, checked as before; a map gains it. This revises the
  record merge, which rejected computed keys on a record type.
- **A literal of constant string keys sets the fields directly,** compiled to
  the copy without building the map, so it runs as fast as `copy` did. On a
  value that is neither a struct nor a map, it is added as `+` adds any map.
- **`+` keeps its precedence.** A field of the result is read through
  parentheses, `(p + { x: 1 }).x`, rather than giving `+` a precedence of its
  own for this use.
- **`copy` is a compile error, `removed_syntax`,** for at least one release,
  with a fix that rewrites it as `+`. The fix adds parentheses where `+`
  would group differently: before `.field`, `[i]`, or a call, after a prefix
  operator, as the right operand of `+` or `-`, and as an operand of `*`,
  `/`, `%`, `:+`, `+:`, or `/>`. `copy` stays reserved meanwhile.

## Consequences

One spelling, and the field checks follow whichever value the checker knows.
A typed program can now reach a runtime failure the record merge ruled out:
merging a runtime map with a key a struct lacks into a value of a record type.
`struct<S>|nil + {...}` is a compile error, where `copy` failed only when the
value was `nil`. A record-typed variable merged into a struct,
`p + changes` with `changes` holding `{zz: 1}`, is checked when it runs, not
by the checker.

## Migration

Every `copy` in the repository was rewritten with the fix (389 uses), none
needing parentheses. Programs elsewhere get the `removed_syntax` error and its
fix. Bytecode is unchanged: `+` of a literal compiles to the same copy.
