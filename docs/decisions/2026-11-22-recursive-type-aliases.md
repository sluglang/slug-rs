# Recursive type aliases

## Status

Accepted; implementation planned in
[Toward a Statically Typed Slug](../planning/typed-slug.md), stage 1.

## Context

Type aliases are transparent and recursive definitions are source errors.
Trees and JSON values need recursive types: `slug.fmt.json` produces values
that are `nil`, booleans, numbers, strings, lists of such values, or maps of
them, and a tree's children are trees. A nominal struct or enum can refer to
itself, since its identity breaks the cycle, but a JSON value is not a struct,
and wrapping one in a schema changes the value.

## Decision

An alias may refer to itself, directly or through other aliases, when its
recursion is contractive and regular.

```slug
type Json = nil|bool|num|str|list<Json>|map<str, Json>
type Tree<T> = {value: T, children: list<Tree<T>>}
```

- **Contractive.** Every reference in a cycle of aliases is inside a type
  constructor: `list<>`, `map<>`, a tuple, a record field, or a function
  type. `type Loop = Loop|nil` and `type A = B; type B = A` are source
  errors: they describe no value.
- **Regular.** A generic alias in a cycle refers to itself, and to the other
  aliases in the cycle, with the same type parameters in the same order.
  `type Tree<T> = {value: T, children: list<Tree<T>>}` is allowed;
  `type Nest<T> = {inner: Nest<list<T>>}` is a source error. This keeps type
  comparison decidable.
- **Comparison.** The checker compares recursive types coinductively: two
  types conform when assuming they do leads to no contradiction.
- **Diagnostics** name a recursive type by its alias, not by expanding it.
- **Nominal types** may always refer to themselves, in field and payload
  types, as they may now.
- **At run time** a recursive type is checked like any other: once per value,
  walking the value, as [the soundness model](2026-11-22-transient-soundness-model.md)
  sets out.

## Consequences

- JSON values, trees, and other recursive data can be typed without schemas.
- The checker needs a coinductive comparison with an assumption set.
- Polymorphic recursion through aliases is not supported.

## Migration

None: recursive aliases were errors and become valid; other aliases are
unaffected.
