# Payload-bearing and generic enum variants

## Status

Accepted; implemented on 2026-11-22
([payload cases](../language/nominal-types.md#payload-cases)). The pattern
form is the whole-pattern type constraint only; no constructor-shaped pattern
was added.

## Supersedes

[Named closed variants](2026-08-29-named-closed-variants-superseded.md).

## Context

Enums are fieldless today: `enum SeekFrom { Start Current End }`. A domain
whose cases carry data, such as a result that is a value or an error, is
modelled with separate schemas and a union, `struct<Ok>|struct<Err>`, which
repeats the domain at every annotation and names no closed set of cases.
[Named closed variants](2026-08-29-named-closed-variants-superseded.md)
proposed record-shaped variants, without generics or positional payloads,
and was deferred. Typed Slug needs generic ones, such as `Result<T, E>`.

## Decision

An enum case may carry a payload of named fields, in braces, and an enum may
take type parameters:

```slug
enum Result<T, E> {
  Ok { value: T }
  Error { error: E }
}

val parsed: Result<num, str> = Result.Ok { value: 3 }

match parsed {
  {value}: Result.Ok => value
  {error}: Result.Error => 0
}
```

- **Cases.** A case is fieldless, as now, or has a braced list of fields
  with the struct schema rules for types, defaults, and required fields. An
  enum may mix both. Cases are separated as they are now.
- **Construction.** A payload case is constructed as a struct is, qualified
  by its enum: `Result.Ok { value: 3 }`. Type arguments are inferred from the
  fields and the expected type, or given by an annotation.
- **Identity.** `Result` is a closed, nominal parent type. Each case is a
  distinct nominal variant type assignable to it; a variant of another enum
  is not. At run time each payload case is a distinct schema identity under
  the enum's namespace, so construction, field reads, copies, equality, and
  schema-identity matching reuse struct behavior.
- **Patterns.** The initial pattern form is the whole-pattern type
  constraint, `{value}: Result.Ok`. A constructor-shaped pattern, such as
  `Result.Ok { value }`, is decided with the implementation.
- **Coverage.** Closed-match coverage includes payload cases: an unhandled
  case is non-exhaustive, and a repeated one unreachable.
- **Payloads are named.** There are no positional payloads (`Some(T)`). They
  would add a second construction form and new pattern syntax, and their
  main use elsewhere, an optional value, is `T|nil` in Slug, narrowed by nil
  tests. Positional payloads are reconsidered once a clutch has been typed
  with `Result`; a single positional payload could then be sugar for one
  named field.
- **Generics are erased.** Type arguments are not checked at run time, as
  [the soundness model](2026-11-22-transient-soundness-model.md) sets out;
  the variant's identity is.

## Consequences

- Closed domains with data get one nominal annotation, exhaustive coverage,
  and field-level checking.
- Payload cases reuse schemas, so no new value layout is needed.
- Fieldless enums keep their current syntax and behavior.
- Annotation and constraint resolution must handle generic enum types and
  qualified variant names, keeping source spellings in diagnostics.

## Migration

None: existing enums are fieldless and remain valid.
