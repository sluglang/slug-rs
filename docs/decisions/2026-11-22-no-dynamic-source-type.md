# No dynamic source type

## Status

Accepted.

## Context

Gradual type systems usually name the unchecked type, as `dynamic`, `any`,
or `?`, so that a program can say where checking stops. Slug's checker has a
private `unknown` state for what it cannot type, and the source type
`any|nil` for every value, which it treats dynamically for operations. Moving
to [sound gradual typing](2026-11-22-annotations-are-guarantees.md), with
strict typing everywhere as the end goal to evaluate, raised whether Slug
needs a `dynamic` annotation.

A `dynamic` annotation would serve only the transition: it marks code as
deliberately unchecked, which unannotated code already is. A strictly typed
language needs no such escape hatch either: a sound language handles a value
of unknown type with a top type that code must narrow before relying on
more.

## Decision

Slug adds no dynamic source type.

- During gradual typing, unannotated code is the dynamic part of a program.
  What the checker cannot type there stays unchecked until it reaches an
  annotated place, where it is checked.
- Under strict typing, if it is adopted, every value is typed by inference or
  annotation. Data whose type is not known, such as a parsed JSON value, a
  host value, or a `describe` result, has the top type `any|nil`, and code
  narrows it with match type constraints and nil tests before relying on
  more. Whether operations on `any` stop being dynamic in strict modules is
  specified with strict modules.
- The checker's `unknown` remains private.

## Consequences

- No new annotation to learn, and no second top type.
- An annotation cannot leave part of a type deliberately unchecked, as
  `map<str, dynamic>` would; it says `any|nil` and narrows, or the place
  stays unannotated.
- Strict modules must state how `any` and `any|nil` behave for operations,
  since today they do not produce static diagnostics.

## Migration

None.
