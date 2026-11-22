# Structural record types

## Status

Accepted; implemented on 2026-11-22 ([Record Types](../language/record-types.md)),
except the boundary checks of [stage 6](../planning/typed-slug.md).

## Context

Most Slug programs pass maps as records: Crawl's world, players, and
regions are maps with fixed string keys. The type system can say `map<str,
V>`, which promises nothing about which keys are present or what each holds,
or `struct<S>`, which names one schema. Neither describes "anything with a
`name` that is a string", which is what a function that reads `.name` needs.

## Decision

A record type lists fields and their types:

```slug
type Named = {name: str}
type Player = {name: str, hp: num, guild: str|nil}
```

`{field: Type, ...}` is a type annotation form, written where any
annotation may be. A field name is an identifier or a string literal.

- **Conformance.** A map conforms when, for each field, its value at that
  string key conforms to the field's type. A struct conforms when, for each
  field, its schema has that field and its value conforms. Fields are named
  by string keys only.
- **Width.** A record type constrains only its fields. A value with more
  fields, or a map with other keys of any kind, conforms: `{name: str, hp:
  num}` conforms to `Named`. There are no exact record types.
- **Absent fields are `nil`.** There are no optional fields. A map without a
  key reads `nil` there, so a field of type `T|nil` accepts a map that lacks
  it, and a field whose type excludes `nil` requires it.
- **Depth.** Field types are covariant, as values are immutable:
  `{pos: {x: num}}` accepts any value whose `pos` conforms to `{x: num}`.
- **Relation to other types.** A record type is structural, with no nominal
  identity. `map<str, V>` does not conform to a record type whose fields
  exclude `nil`, since it promises no key; it conforms to `{field: V|nil}`.
  `struct<S>` conforms to a record type when `S`'s fields do.
- **Checking.** Field reads, destructuring, map and struct construction,
  and copies are checked against a record type where the checker knows it,
  and a record type narrows in match type constraints. At run time a record
  type is checked [deeply, once per value](2026-11-22-transient-soundness-model.md).

## Consequences

- Functions over map records get field-level checking without converting
  programs to schemas.
- A record type says nothing about a value's other fields, so it cannot give
  a value a fixed layout; layout work for records is separate.
- `{` begins a record type in annotation position, where it was unused.
- Generic record types come from [generic aliases](../planning/typed-slug.md#language-features),
  such as `type Box<T> = {value: T}`.

## Migration

None: record types are new.
