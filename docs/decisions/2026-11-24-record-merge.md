# Merging into a record type

## Status

Accepted; implemented on 2026-11-24 ([Record Types](../language/record-types.md)).

Revised by [`+` replaces `copy`](2026-11-28-plus-replaces-copy.md): a
merge with computed keys or a runtime map is now allowed on a record type.

## Context

Programs update map records with `+`: `empire + {wars: wars}`. A
[structural record type](2026-11-22-structural-record-types.md) describes
a map or a struct with its fields, and `+` on a struct was a runtime error,
so the checker rejected `+` on a record type. Typing Crawl's state as record
types would have rejected about a hundred such merges, and any program that
types its map records meets the same wall. `copy` already worked on both.

## Decision

`r + {k: v, ...}`, where `r` has a record type and the right operand is a
map literal whose keys are distinct string constants, sets fields, as
`copy` does:

- **Each key must be a field the record type lists.** A struct the type
  describes has those fields; it may lack any other. The checker reports a
  key the type does not list, naming it.
- **The result has the record type, with each set field of its new value's
  type,** as `copy` gives. A declared result or parameter then checks it.
- **At runtime, `struct + map` is a copy of the struct with the map's
  fields replaced,** checked against the schema's field annotations as
  `copy` is. A key the struct lacks fails as `copy` fails. `map + map` is
  unchanged.

Merging a map whose keys are computed, `r + {[k]: v}`, and removing a key,
`r - k`, stay rejected on a record type: they are a dictionary's operations,
and a struct cannot lose a field.

## Consequences

A typed program cannot reach the runtime failure: a value of a record type
is a map, which the merge updates as before, or a struct with at least the
type's fields, which the copy replaces. An untyped `struct + map`, an error
before, is now a copy. Map records keep their idiom, so typing a program's
state needs record types, not rewrites.

## Migration

None needed: every program that compiled still does. A program that relied on
`struct + map` failing now gets the copy.

## Alternatives

- **Rewrite merges as `copy`.** No language change, but every program that
  types its map records would need it, and `+` is what Slug programs write.
- **A record type that describes only maps,** `map {k: T, ...}`, on which `+`
  and `-` work as on maps and structs do not conform. Explicit and with no
  runtime change, but it adds syntax and a choice between two kinds of record
  type each time one is written.
