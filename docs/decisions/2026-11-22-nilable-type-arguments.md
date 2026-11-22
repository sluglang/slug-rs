# Nilable type arguments

## Status

Accepted; implemented on 2026-11-22.

## Amends

[Make `any` the non-nil top type](2026-08-28-non-nil-any-type.md), its
section "Bare generics remain non-nil".

## Context

A bare type parameter ranged over non-nil types: a call could not infer `T`
as `str|nil`, nor give it explicitly. That was harmless while generics were
functions over lists and maps, where `T|nil` marks a position that may be
`nil`. Generic schemas and enums made it a wall. `Result.Ok { value: name }`
with `name:str|nil`, a `Box<str|nil>`, or `or(result, nil)` over a
`slug.std` `Result` were all source errors, though each describes an
ordinary value, and `T|nil` inside the declaration cannot express them: the
type argument comes from the field.

## Decision

- **A type argument may include `nil`.** Inference may give `T` a nilable
  type, and an explicit type argument may be one: `identity<str|nil>(name)`.
- **A body cannot assume `T` is not `nil`.** Inside a generic declaration, a
  value of type `T` does not conform to `any`, since `T` may stand for
  `str|nil`; it conforms to `T`, `T|nil`, and `any|nil`.
- **`T|nil` positions still take the non-nil part.** Passing `str|nil` to
  `list<T|nil>` infers `T` as `str`, as before.
- **`nil` widens an inferred argument.** When one position has fixed `T` and
  another gives `nil`, `T` becomes nilable: `or(result, nil)` for a
  `Result<map, E>` is a `map|nil`. Any other disagreement between positions
  is still an error.

## Consequences

- Generic schemas and enums can carry nilable values, and `or(nil)` turns a
  `Result` into a `T|nil`.
- A generic function whose body passed a `T` where `any` is expected is now
  a source error; none in the repository did.
- Diagnostics show a type parameter as `T0`, `T1`, which matters more now
  that bodies see these errors.

## Migration

None in the repository. A program outside it whose generic body relied on
`T` being non-nil gets a source error there, and writes `T` where it wrote
`any`, or narrows with a nil test.
