# Transient soundness with checked immutable collections

## Status

Accepted; implementation planned in
[Toward a Statically Typed Slug](../planning/typed-slug.md), stage 6; specified in
[Annotation checks](../language/annotation-checks.md).

## Context

[Annotations are guarantees](2026-11-22-annotations-are-guarantees.md)
requires a runtime check where a value of unproven type reaches an annotated
place. Gradual type systems differ in what such a check covers:

- Deep, guarded checks (Typed Racket) verify a whole value and wrap
  functions and mutable data in contracts. Typed code can trust everything,
  but wrappers cost time, change identity, and repeat deep checks.
- Transient checks (Reticulated Python) verify a value's first-order shape at
  each boundary and at each use of an element or result. They are cheap and
  wrapper-free, and typed code can trust the tag of the value in hand.
- Concrete types (Thorn, StrongScript) make values carry their static type,
  so untyped values cannot enter typed code without being rebuilt.

Two properties of Slug change the trade-off. Lists, maps, bytes, structs, and
enum values are immutable, so a value that once satisfied `list<num>` always
does. And every value is tagged, so a first-order check is one compare,
which the native tier already makes as a guard.

## Decision

Slug's runtime checks are transient, except that immutable collections are
checked deeply and the result kept.

- **First-order types.** A value reaching `nil`, `bool`, `num`, `str`,
  `bytes`, `schema`, a resource type, an enum or enum variant, or
  `struct<S>` has its tag or nominal identity checked. A union checks that
  the value satisfies a member.
- **Immutable collections.** A value reaching `list<T>`, `map<K, V>`, a
  tuple type, or a [record type](2026-11-22-structural-record-types.md) is
  checked deeply the first time, and the result is kept with the value, so
  that a later check of the same value against the same type, and a read of
  its elements in typed code, cost no further scan. A collection that typed
  code builds from values of proven types is checked by construction.
- **Functions.** A value reaching a function type has its callable tag
  checked. Its parameters and result are checked transiently, with no
  wrapper: an annotated function checks its arguments on entry when a caller
  is not proven typed, and a typed caller checks a result whose type it
  could not prove. Function identity and equality are unchanged.
- **Tasks and channels.** A value reaching `task<T>` or `chan<T>` has its tag
  checked; a payload is checked when it is received into an annotated
  binding, not when it is sent.
- **Generic parameters are erased.** A type parameter is not checked: in
  `fn<T>(x:T):T`, nothing typed can depend on `T` beyond what its uses check.
- **Type arguments of structs and enums are checked.** A value reaching
  `Box<num>` or `Result<num, str>` is checked deeply, as an immutable
  collection is: each field against its type with the type arguments
  substituted, once, and the result kept. A generic alias is checked as what
  it stands for. (Amended 2026-11-22: the type arguments were to be erased,
  but then a field read from a struct of unproven contents would have a type
  nothing enforced.)

Typed code may rely on the first-order type of every value it holds, and of
every element it reads from a collection it holds. The representation that
keeps a check's result with a value is an implementation detail.

## Consequences

- The native tier can keep proven `num` values unboxed and drop guards the
  checker made redundant, within what these checks cover.
- No wrappers: values keep their identity and equality across boundaries.
- A function's parameter types are guaranteed only inside it, through its
  entry check; a caller that passes a function value cannot rely on how it
  will be called.
- Deep checks of large collections that cross many boundaries could be
  costly; keeping the result bounds this, and stage 6 measures it before the
  native tier depends on the checks.
- Checking type arguments makes a field read from a generic struct, or a
  payload read from an enum value, as reliable as an element read from a
  list.
- Match type constraints already check value categories, nominal types,
  unions, and `list<T>`/`map<K, V>`; boundary checks share that machinery,
  extended to tuples, records, and functions' tags.

## Migration

None until stage 6; then as [Annotations are
guarantees](2026-11-22-annotations-are-guarantees.md) describes.
