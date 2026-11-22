# Annotations are guarantees

## Status

Accepted; implementation planned in
[Toward a Statically Typed Slug](../planning/typed-slug.md), stage 6.

## Amends

[Unify source type checking](2026-09-05-unified-source-type-checking.md),
[Select overloads from static callable signatures](2026-08-28-static-overload-selection.md),
[Make `any` the non-nil top type](2026-08-28-non-nil-any-type.md), and
[Propagate known calls and nil partitions in body-derived inference](2026-09-20-body-derived-call-and-nil-partition-inference.md),
where they state that annotations add no runtime validation.

## Context

An annotation today neither coerces nor checks a value, and the checker
reports only contradictions it can prove. A value from code the checker could
not type may reach `name:str` holding anything, and no error occurs until
something uses it as a string, if anything does. So no part of the system can
rely on a type: the compiler cannot drop a check, the native tier cannot keep
a proven `num` unboxed, and a reader cannot trust a signature. And an
annotation may describe less than its code produces: a function annotated
`:num` whose body may return `num|str` is accepted, because it may return a
`num`.

## Decision

Two rules replace "annotations add no runtime validation". The rule that the
checker reports only provable contradictions stays, for unannotated code.

**An annotation is a guarantee.** Where a value whose type the checker could
not prove reaches a place with an annotated type, a runtime check verifies it
there. Such a value comes from unannotated code, directly or through a
binding, element, field, or result whose type the checker could not
determine. The places are: an annotated binding's initializer, an assignment
to an annotated `var`, an argument to an annotated parameter, the result of
an annotated function, an annotated struct field's value, and a value
received into an annotated binding from a channel or task. A failed check is
a checked runtime error at that place, reporting the annotation's location,
the expected type, and the value's actual type. Annotations still never
coerce.

**An annotation is accurate and complete.** The type the checker infers for
what an annotation describes must conform to it. A body that may return
`num|str` under `:num`, or an initializer of type `str|nil` for a binding
annotated `str`, is a source error. Only the part of an inferred type that
comes from unannotated code, which the checker could not determine, is left
to the runtime check.

Within code whose types the checker proved, the compiler and the runtime may
rely on every proven type. What a check covers, and so how far a type may be
relied on, is set by [the soundness model](2026-11-22-transient-soundness-model.md).

## Consequences

- Types become usable by the compiler and the native tier, within what the
  soundness model checks.
- A program whose annotations are wrong, but which never used a wrong value
  in a way the VM rejects, now fails at the annotation, or does not compile.
  This is the intended effect.
- Annotated code may need narrowing it did not need before: a value of type
  `str|nil` must be narrowed to `str` before it reaches a `str` annotation.
- Boundary failures need a diagnostic code and the expected and actual types,
  consistent with [Diagnostic Documents](../reference/diagnostics.md).
- The cost of the checks must be measured on every benchmark before anything
  relies on them.
- Inferred types need no rule of their own: what inference proves holds, and
  what it cannot prove stays dynamic, as unannotated code is.

## Migration

Annotations in the repository's programs, clutches, and standard library are
checked against the new rules in stage 6, and wrong ones corrected, before
the rules apply by default. A program outside the repository with a wrong
annotation gets a source error, or a runtime error at the annotation instead
of at a later use.
