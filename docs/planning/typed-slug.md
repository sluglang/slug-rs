# Toward a Statically Typed Slug

This document plans moving Slug from optional static checking to sound
gradual typing, with strict typing everywhere as the end goal to evaluate. It
is a dependency-ordered roadmap, not a source-language specification: each
stage must state its user-visible rules in the documents under
[`language/`](../language/README.md), and record durable choices in
[`decisions/`](../decisions/), before it is implemented.

It builds on [the type-system plan](type-system-plan.md), whose stages 1–4
are essentially done, and on [the native tier](native-tier.md), which is the
main runtime consumer of type facts.

## Purpose

Slug's types today describe programs but promise nothing: an annotation
neither coerces nor checks a value, and the checker reports only
contradictions it can prove. That keeps every dynamic program valid, but it
means no part of the system may rely on a type. The compiler cannot drop a
check, the native tier cannot keep a `num` unboxed, and a reader cannot trust
that `name:str` holds a string.

The goal is types that hold: where code is typed, its types are guaranteed at
run time, so the compiler, the native tier, and readers can depend on them.
Untyped code stays valid, and the boundary between the two is checked.
Whether to then require types everywhere is a separate decision, taken on
evidence once gradual typing works (see [Strict typing as the end
goal](#strict-typing-as-the-end-goal)).

## What exists

Assessed on 2026-11-22.

| Area | Exists | Gap |
| --- | --- | --- |
| Annotations | Bindings, parameters, results, foreign declarations, struct fields. | Non-coercive and unchecked at run time. |
| Unions and nil | `A\|B`, `str\|nil`, `any` (non-nil top), `any\|nil`. | No source-visible dynamic type; the checker's `unknown` is private. |
| Generics | `fn<T>` with call-site inference and explicit application. | No generic aliases, structs, or enums; no bounds. |
| Narrowing | Nil tests, `&&`/`\|\|`, match constraints, branch joins. | Broader predicates stay conservative. |
| Records | Nominal schemas, `struct<S>`, checked construction, copies, field reads. | No structural record types; maps (most programs' records) are `map<K, V>` only. |
| Enums | Fieldless, nominal, closed-match coverage. | No payloads; [the variants ADR](../decisions/2026-08-29-named-closed-variants-superseded.md) is proposed, deferred. |
| Aliases | Transparent, exported, non-recursive. | Not generic, not recursive. |
| Inference | Body-derived parameter facts, return unions, structural `fn` types, generic arguments at calls. | Local and partial; a binding keeps a value's dynamic part ([decision](../decisions/2026-10-18-a-binding-keeps-a-values-dynamic-type.md)). |
| Runtime type tests | Match constraints test value categories, nominal types, unions, and `list<T>`/`map<K, V>` recursively. | Function types, tuples, task/channel payloads, and generic parameters are not runtime-checkable. |

Two properties of the runtime make soundness cheaper than in most dynamic
languages. Lists, maps, bytes, structs, and enum values are immutable
([invariants](immutable-collection-value-redesign.md#invariants)), so a value
that once satisfied `list<num>` always does; a check's result can be kept with
the value. And every value is tagged, so a first-order check is one compare,
which the native tier already performs as a guard.

## The rule under review

[The type-system plan's invariants](type-system-plan.md#scope-and-invariants)
and [unified source type checking](../decisions/2026-09-05-unified-source-type-checking.md)
fix three rules:

1. Annotations neither coerce nor add runtime validation.
2. Only provable contradictions are reported; incomplete information widens
   conservatively rather than rejecting a program.
3. No runtime type tags, coercion, or reflection are introduced incidentally.

Rule 3 should stay in spirit: values are already tagged, and soundness needs
no new representation, only checks. Rule 1 is the one to replace, because it
is exactly what makes types unreliable. Rule 2 stays for unannotated code,
which a gradual checker must accept when it cannot type it, but not for
annotated code, as the second rule below says:

> **Proposed rule.** An annotation is a guarantee. Where a value whose type
> the checker could not prove flows into a place with a declared or inferred
> static type, a runtime check verifies it there, and a failed check is a
> checked runtime error naming that place. Within typed code, the compiler
> and runtime may rely on every type the checker proved. Annotations still
> never coerce.

> **Proposed rule.** An annotation is accurate and complete. The type the
> checker infers for what an annotation describes must conform to it: a
> function annotated `:num` whose body may return `num|str` is a source
> error, though today it is accepted because it may return a `num`. Only a
> part of the inferred type the checker could not determine, coming from
> unannotated code, is left to a runtime check.

Inferred types need no rule of their own: what inference proves holds, so it
needs no check, and what it cannot prove stays dynamic, as unannotated code
is.

This changes behavior for one kind of program: one whose annotations are
wrong but which never ran into the wrong value's use. Such a program fails
earlier, at the annotation. That is the intended effect, and the migration
stage measures how often it happens in the repository's own code.

## The soundness model

Gradual type systems differ in what a boundary check covers. The choice
decides the cost of soundness and how far the compiler may trust a type.

| Model | Boundary check | Trust inside typed code | Cost |
| --- | --- | --- | --- |
| Erasure (today) | None | None | None |
| Deep / guarded (Typed Racket) | Full value; functions and mutable data wrapped in contracts | Complete | Can be very high: wrappers, repeated deep checks, lost identity |
| Transient (Reticulated Python) | First-order shape at each boundary and at each use of an element or result | First-order: the tag of what is in hand | Small, spread across uses |
| Concrete (Thorn, StrongScript) | Values carry their static type; check is a tag compare | Complete for concretely typed values | Needs typed constructors; untyped values cannot enter concrete types |

**Proposal: transient checks, with deep checks for immutable collections.**

- A first-order check at each boundary: a value entering a `num`, `str`,
  `struct<S>`, enum, or resource type has its tag or identity checked.
- Immutable collections are checked deeply, once: `list<num>` scans its
  elements the first time a value crosses into it, and the result is kept
  with the value (a summary of its element kinds), so later crossings and
  every element read are free. A value built in typed code from typed
  elements is born checked.
- Functions are checked transiently: a typed function checks its arguments
  on entry when its caller is not proven typed, and a typed caller checks a
  result it could not prove. No wrappers, so function identity and equality
  are unchanged.
- Generic parameters are erased: `fn<T>(x:T):T` checks nothing about `T`,
  since nothing typed can depend on it beyond what its uses check.
- Channel and task payloads are checked when a value is received into a
  typed binding, not when sent.

This gives the native tier what it needs, the tag of the value in hand, at
the cost of a compare it often already makes. Whether deep collection checks
cost too much on real programs is measured in stage 6 before they are
relied on.

**No dynamic type in the source.** Unannotated code is the dynamic part of a
program: what the checker cannot type there stays unchecked until it meets a
static type, where it is checked. A `dynamic` annotation would only be a
bridge to strict typing, and unannotated code already is one. Strict typing
needs no escape hatch of that kind either: data whose type is not known, such
as a parsed JSON value, a host value, or a `describe` result, has the top type
`any|nil`, which code narrows with match constraints and nil tests before it
relies on more, as sound languages handle an unknown value. The cost is that
an annotation cannot leave part of a type deliberately unchecked
(`map<str, dynamic>`); it says `any|nil` and narrows, or the place stays
unannotated.

## Language features

These are the features the plan adds, in dependency order. Stages 1–5 are
checker-only and ship under today's rules, so they are useful before
soundness lands.

- **Generic type aliases.** `type Optional<T> = T|nil`, `type Pair<T, U> =
  [T, U]`. Transparent, as aliases are now; parameters are erased. An alias
  may be recursive when every reference to itself is inside a type
  constructor (`list<>`, `map<>`, a record field, a function or tuple type)
  and a generic one refers to itself with the same parameters: `type Json =
  nil|bool|num|str|list<Json>|map<str, Json>` is allowed, `type T = T|nil`
  and `Tree<T>` referring to `Tree<list<T>>` are not. These rules keep
  checking decidable; the checker compares recursive types coinductively, and
  diagnostics print the alias name. A nominal struct or enum may always refer
  to itself.
- **Structural record types.** `type Named = {name: str}` describes any value
  with at least a `name` field holding a string: a map with that string key,
  or a struct whose schema has that field. Open (width) subtyping, so
  `{name: str, age: num}` conforms to `Named`. Fields are named by string
  keys only. There are no optional fields: `age: num|nil` says a field may be
  absent, since a map without the key reads `nil` there, as it does now.
  This is the feature real Slug code needs most, since Crawl and most
  programs pass maps as records.
- **Generic nominal types.** `val Pair = struct<T, U> { first: T, second:
  U }`, and generic schemas behind `struct<S>`, as `struct<Pair<num,
  str>>`. Shares type-parameter machinery with enums.
- **Payload-bearing and generic enums.** `enum Result<T, E> { Ok { value: T }
  Error { error: E } }`, constructed as `Result.Ok { value: 3 }`. This
  revives [the variants ADR](../decisions/2026-08-29-named-closed-variants-superseded.md),
  whose record-shaped payloads it keeps, for consistency with struct
  construction, and extends with generics. A variant is a distinct nominal
  identity, coverage is exhaustive, and construction reuses schemas at run
  time. Payloads are named only. Positional payloads (`Some(T)`) would add
  a second construction form and new pattern syntax; their main use
  elsewhere, `Option`, is `T|nil` in Slug already, with nil narrowing doing
  what matching `None` does. They are revisited once a clutch is typed with
  `Result`, when a single positional payload could become sugar for one
  named field.
- **Inference.** Bidirectional, local inference: types flow down from
  annotations and expected types and up from expressions, with generic
  arguments solved per call. Global Hindley–Milner inference is not the
  target: unions, structural records, and nil narrowing make principal types
  awkward, and local inference gives errors at the expression that caused
  them. The measure is how many annotations a typed program needs, not
  whether none are.

Variance follows from immutability: `list<T>`, `map<K, V>`, records, and
enum payloads are covariant, and function parameters contravariant. No
variance annotations are planned.

## What the runtime gains

Measured so far, so expectations are realistic:

- **Numeric code gains most.** The native tier's prototype ran `fib(32)` in
  24 ms with boxed values and 8.2 ms with typed integers (interpreter 242
  ms). Proven `num` lets native code keep numbers in registers and drop tag
  checks.
- **Data-heavy code gains little from types alone.** In a hand-compiled Crawl
  function, 92% of native time was runtime work, chiefly map reads and copies.
  Struct reads were about 0% of Crawl's reads, because it uses maps. Records
  with a known shape could get a fixed layout, but only once typed code
  builds them as such: structural record types alone do not change a map.
- **The `typed-*` benchmark twins** ran the same instruction count as their
  untyped twins: annotations do nothing at run time today. They are the
  first measure of stage 7.

So the first payoff is correctness and diagnostics everywhere, and speed in
numeric code; layout work for records is a later, separate step.

## Strict typing as the end goal

Strict typing everywhere means every module must type-check before it runs:
inference or annotations type every value, and data of unknown type is
`any|nil`, narrowed before use. Unannotated code stays allowed wherever
inference types it. It is
worth serious evaluation, not adoption by default, because its benefits and
costs are both large.

**Benefits.** Every value's type is known statically, so boundary checks
disappear, leaving the narrowing that code writes itself; the native tier can specialize all code;
tooling (completion, `describe`, typed holes in [the agent-facing
direction](agent-facing-direction.md)) becomes complete; and a reviewer can
trust every signature.

**Costs.** Every existing program, clutch, and the standard library must be
typed. Agents write much of Slug's code, and type errors cost them attempts:
[a binding keeps a value's dynamic type](../decisions/2026-10-18-a-binding-keeps-a-values-dynamic-type.md)
was decided because an agent hit a spurious `expected list, got any|nil` and
had to annotate two functions to continue. Strictness multiplies that
friction unless inference and diagnostics are good enough to remove it.

**Preconditions.** Strict typing is worth adopting only when, measured on
the repository's programs and the agent evaluation projects:

- typed programs need few annotations beyond function signatures (target to
  set in stage 8, such as one per exported function);
- every type error carries the expected and actual types, and a fix where
  one is unambiguous;
- the standard library and shipped clutches are fully typed;
- agents complete the evaluation projects with no more fix attempts than
  they need today;
- the runtime gain is measured, not assumed.

**Path.** Gradual typing first, then opt-in strict modules (a module
declares it is strict, and everything in it must type), then strict as the
default once the preconditions hold. Each step is reversible until the
default changes.

## Stages

Each stage lands its normative documents, grammar changes, support matrix,
tests, and changelog in the same change, as the [docs rules](../AGENTS.md)
require.

### 0. Decisions

- [x] [Annotations are guarantees](../decisions/2026-11-22-annotations-are-guarantees.md):
  checked where unannotated values reach them, and accurate and complete,
  keeping rule 2 for unannotated code and rule 3.
- [x] [Transient soundness with checked immutable collections](../decisions/2026-11-22-transient-soundness-model.md):
  what is and is not checked.
- [x] [No dynamic source type](../decisions/2026-11-22-no-dynamic-source-type.md).
- [x] [Structural record types](../decisions/2026-11-22-structural-record-types.md).
- [x] [Payload-bearing and generic enum variants](../decisions/2026-11-22-payload-enum-variants.md),
  superseding named closed variants.
- [x] [Recursive type aliases](../decisions/2026-11-22-recursive-type-aliases.md).
- [x] Point [the type-system plan](type-system-plan.md)'s invariants here, and
  mark the records these amend.

### 1. Generic type aliases

- [x] Specify parameters, expansion, arity errors, export, and import
  ([aliases](../language/nominal-types.md#aliases)).
- [x] Specify recursive aliases: contractive and regular, compared
  coinductively ([aliases](../language/nominal-types.md#aliases)).
- [x] Implement generic aliases in the checker; they stay transparent and
  erased.
- [x] Implement recursive aliases. A recursive type is not yet
  runtime-checkable, so it cannot be a match type constraint; stage 6's
  boundary checks need the same recursive check.

### 2. Structural record types

- [x] Specify `{field: Type}` types: which maps and structs conform, width
  subtyping, absent fields as `nil`, and how they meet `map<K, V>` and
  `struct<S>` ([record types](../language/record-types.md)).
- [x] Check field reads, construction, copies, and destructuring against
  them; narrow with match constraints. A map literal with constant string
  keys has a record type, which is also its map type.
- [x] Make record types runtime-checkable for match constraints, so stage 6
  can check them at boundaries.

### 3. Generic nominal types

- [x] Specify generic schemas and `struct<S<T>>`, construction with
  inferred type arguments, and field types under substitution
  ([generic schemas](../language/structs.md#generic-schemas)).
- [x] Implement; type arguments are erased at run time, so a match
  constraint names a generic schema only as `struct<S>`.

### 4. Payload-bearing and generic enums

- [x] Specify [payload and generic variants](../decisions/2026-11-22-payload-enum-variants.md)
  in the language documents ([payload cases](../language/nominal-types.md#payload-cases)),
  and decide constructor-shaped patterns: none for now; the whole-pattern
  constraint `{value}: Result.Ok` is the form.
- [x] Implement variants as distinct schema identities under the enum's
  namespace, as the ADR proposed; extend coverage to payload variants.
- [x] Type `Result`-style returns in one clutch as the test of the design,
  and revisit positional payloads then. Crawl's store (`slug.games.crawl.store`)
  now returns `Result<T, StoreError>` from `open` and `read`, with `enum
  StoreError { Missing, WrongLayout { found: num, expected: num } }`.
  Findings:
  - The trial found a checker bug: a payload case constraint on a subject
    of type `Result|nil` was reported as unable to match, since general
    union coverage did not know a case is part of its enum. Fixed; such a
    subject, as with fieldless enums in a union, needs a catch-all case,
    since no type says "`Result` without `Ok`".
  - Taking a value out is the most common use and the wordiest: every
    caller but one wrote the same two-armed `match`, `{value}:
    store.Result.Ok => value` and `{error}: store.Result.Error => throw
    ...`. A way to unwrap or propagate an error is the clearest gap.
  - `Result.Ok { value: db }` is long for a one-field payload, and `value`
    names nothing; a single positional payload as sugar for one named
    field would read better in construction. Patterns read well as they
    are. The named form suited `WrongLayout { found, expected }`.
  - Each module's `Result` is its own type, so two modules' results do not
    mix; a shared `Result` belongs in the standard library once the design
    settles.
  - `Result` replaced a `has` check before `read` in the web server, so the
    absence of a world is handled where the world is read. Crawl's other
    "nothing here" returns, such as `addToPack`'s `str|nil`, are better as
    `T|nil`, as the decision record expected.

  Decided: `Result` moved to `slug.std`, with `or`, `orElse`, and `orThrow`
  read through the pipeline (`store.read(db) /> or(nil)`); no positional
  payloads and no `?` operator. Making that work took three checker
  changes: [nilable type arguments](../decisions/2026-11-22-nilable-type-arguments.md),
  inference that is contravariant in function parameters, and destructured
  imports that bring a named type.

### 5. Inference

- [x] Specify bidirectional checking: expected types for literals, lambdas,
  and generic calls; where a type flows down. A function literal takes its
  parameters' types from the callee's signature or an annotation, a match
  body's patterns from the parameters, and an annotated call's result fills
  in undetermined type arguments.
- [x] Infer generic arguments from expected types as well as arguments.
- [x] Measure annotations needed on the repository's programs before and
  after (below).
- [ ] Deferred: compare function types with a covariant result and
  contravariant parameters, rather than equal pairs. Inference already does,
  which is enough for `each`. As a type relation it changes the outcome of
  the trial check that infers a `var`'s type, which breaks a `var` and `val`
  overload pair in `module_loader`'s `selected_inferred_alternative_rejects_a_replaced_live_binding`.
- [ ] Deferred: a function literal passed to a function declared later in
  its module gets no expected types, since the callee's signature is not yet
  known; `slug.std` declares `reduce` before `flatMap` for this reason.

Survey, 2026-11-22, before any inference work. Each expression the checker
saw was classed by its type: known, partial (a part is unknown, as `list`
or `map<str, unknown>`), or unknown (`unknown` or `any|nil`).

| Programs | Expressions | Known | Partial | Unknown | Unknown unannotated parameters |
| --- | ---: | ---: | ---: | ---: | ---: |
| Crawl | 30,600 | 56% | 24% | 20% | 253 of 303 |
| `slug.text.markdown` | 17,766 | 90% | 5% | 6% | 3 of 4 |
| `slug.net.http` | 5,972 | 76% | 10% | 14% | 20 of 20 |
| `slug.text.mustache` | 2,936 | 58% | 23% | 19% | 16 of 16 |
| `slug.core` | 9,230 | 60% | 12% | 29% | 339 of 361 |
| Benchmarks | 16,879 | 61% | 15% | 24% | 415 of 442 |

Where the unknowns come from, most first:

1. **Lambdas passed to untyped higher-order functions.** In Crawl, 241 of
   the 253 unknown parameters belong to a function literal passed as an
   argument, mostly to `map`, `filter`, `reduce`, and `each`. Those take
   `vs:list, f:fn` and return `list`, so neither the lambda's parameters
   nor the result's elements are known. Generic signatures, as
   `fn<T, U>(vs:list<T>, f:fn<U, T>):list<U>`, need bidirectional checking:
   a function literal checked against an expected `fn<R, P...>` takes its
   parameter types from it, and its result infers `U`. This is the main
   inference gain.
2. **Untyped builtins.** Only `chan`, `close`, and `Error` take their
   `slug.builtin` signatures; `len`, `keys`, and the rest return unknown,
   639 times for `len` alone. Typing `len` and `keys` cuts unknown
   expressions in Crawl from 20% to 18%, in http from 14% to 8%, in
   mustache from 19% to 13%. It breaks two programs (`slug.fmt.json`'s
   `trimHigh`, the tinylang benchmark): body-derived inference takes `len`'s
   `str|bytes|list|map` parameter as a constraint on an unannotated
   parameter, which a later slice then contradicts. A parameter type that
   is a union of families should not constrain body-derived inference.
3. **Bare container annotations.** Of Crawl's parameter annotations, 305 of
   1,283 are bare `map`, `list`, `fn`, or `struct`, and 232 of its 467
   result annotations are `map` or `list`; most unknown field reads, 1,833,
   are on a bare `map`. Inference cannot recover these: record types and
   generic schemas can, through the annotation pass of stage 8.
4. **Dynamic data.** `slug.fmt.json` results, SQLite rows, and `import` are
   unknown by nature; stage 6's boundary checks cover them.
5. **Unannotated top-level functions.** In the benchmarks and `slug.core`,
   most unknown parameters are top-level functions that use them only
   dynamically. Local inference does not look at callers, by design.

The order this suggests: type the builtins, with the body-derived fix;
check function literals against expected function types; give
`slug.std`'s higher-order functions generic signatures; then measure again.

Measured again after those three, 2026-11-22: unknown expressions fell in
Crawl from 20% to 17%, in http from 14% to 8%, in mustache from 19% to 13%,
and in markdown from 6% to 4%. Crawl's unknown unannotated parameters fell
from 253 to 153: 150 function literal parameters are now typed, 84 fully.
Most of the 141 still unknown are over values Crawl annotates as a bare
`map` or `list`, so `keys(world.players)` or `region.items` gives nothing
to flow; typing `keys` as `fn<K, V>(m:map<K, V>):list<K>` changed nothing
there. The rest of the gain is the annotation work of stage 8, with record
types and generic schemas, not inference. Typing `slug.std` found one place
in Crawl whose types the code relied on without saying, a `filter` that
removed `nil`s, now written with `compact`, and one wrong annotation.

### 6. Sound boundaries

- [x] Specify where checks occur: binding, argument, result, field, element
  read, receive; the error, with the annotation's location. Specified in
  [Annotation checks](../language/annotation-checks.md). Two decisions were
  amended on the way: struct and enum type arguments are checked, not
  erased, and a dynamic call to an overload set selects by type.
- [x] Record in the checker where checks are needed: a value reaching an
  annotated or inferred type that it is not proven to conform to, by
  `is_proven_assignable_to`, which treats an undetermined part as conforming
  only to a type every value does. Measured over the repository's `.slug`
  files (5,516 sites):

  | Place | Sites | Note |
  | --- | --- | --- |
  | Annotated parameter | 4,525 | 2,009 `num`, 898 `str`, 581 `map`, 475 `list`: nearly all one tag compare |
  | Inferred parameter | 560 | 205 `num`, 142 `str`, 116 `map` |
  | Result | 292 | Mostly values from untyped `foreign` raw functions and unannotated helpers; 152 in Crawl |
  | Assignment | 84 | Most to unannotated `var`s, against their first value's type |
  | Struct field | 38 | |
  | Call through a function value | 10 | |
  | Declaration | 7 | |

  75 sites check against a type with a type parameter, whose check is
  partly erased.
- [x] Close the gaps the survey found before anything relies on the checks:
  - A dynamic value reaching a type parameter in a generic function's
    result, as `fn<T>(x:T):T { helper(x) }`, cannot be checked inside it, so
    the caller must check the instantiated result.
  - A generic higher-order function launders a function value's type:
    `map(xs, g)` with `g:fn<num, str>`, checked only to be a function, gives
    a `list<num>` nothing enforced. A type argument inferred from such a
    value makes the call's result unproven.
  - A `foreign` function's result annotation is not yet recorded as a check.
  - A copy of a struct whose schema the checker does not know needs the
    runtime to check the replaced fields against the schema's annotations.

  Closed in the checker. A function notes whether a value it could not
  prove reaches a type parameter in its result, by
  `is_exactly_assignable_to`, where a type parameter accepts only itself,
  and its signature carries that to callers and importers; a generic call
  then checks its instantiated result, as it does when an argument is a
  function value of unenforced type. A call to a callee checked later has an
  unknown result, which its caller checks already; one whose callee is not
  yet known is settled when the module is checked. A `foreign` result is a
  check where it is declared, and a construction or copy of a struct of
  unknown schema records a check against the schema read when it runs,
  which the VM needs schemas to carry field types for. The survey then
  counts 5,956 sites: 215 `foreign` results, 211 fields of structs of
  unknown schema, mostly copies of values that are maps when they run, for
  which the check does nothing, and 24 call results, 14 more than before. On
  the way, a type
  parameter now conforms to `any|nil`, which it did not: `fn<T>(x:T) {
  show(x) }` was rejected for a `show` with an unannotated parameter.
- [x] Implement the checks in the compiler and the VM, inserting them only
  where the checker recorded one; with native-tier support, since a
  function with an op the tier does not compile runs interpreted. A check
  is the match test a match constraint makes, `_: T`, which leaves the
  value, and on failure one new op, `CheckFailed`, whose message constant
  names the type and place; the native tier ends its block there and the
  interpreter raises the error, so a checked function stays native. Every
  clutch suite passes with the checks. They caught three conformance
  fixtures assigning a map or function to a `var` that began as `0`, which
  the spec's `var` rule forbids, and made `slug.string`'s own element check
  in `sort` and `join` redundant, so it is gone.
- [ ] Finish the checks:
  - A `foreign` function's result, which the VM checks when the host
    returns; and fields of structs of unknown schema, which needs schemas to
    carry field types at runtime.
  - A recovering `defer onerror`'s result.
  - Dynamic overload dispatch by type, as the amended decision requires:
    the binder needs each member's parameter checks.
  - Deep checks of struct and enum type arguments, `Box<num>`, of recursive
    types, and of tuples by position, which are now checked as lists of the
    union of their elements.
  - A schema the check cannot name, as one private to another module, is
    checked only to be a struct: schemas need an identity the runtime can
    compare.
  - The diagnostic code `annotation_mismatch`, the annotation's location,
    and the failing part of a deep check.
- [ ] Keep a checked-element summary with immutable collections, and mark
  values built in typed code as checked.
- [ ] Measure the cost on every benchmark (interleaved PGO A/B, hold-outs
  untouched) and on the clutch suites, before anything relies on the checks.
- [ ] Measure how many of the repository's programs and clutches fail a new
  check, and fix them or their annotations.

### 7. The runtime uses proven types

- [ ] Native tier: keep proven `num` values unboxed and drop guards the
  checker made redundant; start with arithmetic loops.
- [ ] Measure on the `typed-*` twins and on typed versions of Crawl's hot
  functions.
- [ ] Evaluate fixed layouts for records typed code builds, separately.

### 8. Typed libraries and strict modules

- [ ] Type the standard library and shipped clutches; record the
  annotations each needed.
- [ ] Specify opt-in strict modules, where everything must type and unknown
  data is narrowed from `any|nil`.
- [ ] Run the agent evaluation projects in strict and gradual modes and
  compare fix attempts and completion.

### 9. Strict by default

- [ ] Decide against the preconditions above. If they hold, make strict the
  default with a migration path; if not, record why gradual typing stays.

## Risks

- **Boundary cost.** Deep collection checks on large values crossing often
  could cost more than they save. The checked-element summary and stage 6's
  measurements bound this before the native tier depends on it.
- **Breaking wrong annotations.** Programs with incorrect annotations that
  happened to work will fail at the annotation. Stage 6 measures and fixes
  the repository's own first.
- **Agent friction.** More type errors cost agents attempts. Diagnostics with
  expected and actual types, and fixes, are a precondition, not a follow-up.
- **Specification size.** Generics, records, and variants add rules agents
  must learn; the spec's size is itself a design constraint
  ([agent-facing direction](agent-facing-direction.md)).
- **Two semantics during the transition.** Gradual code checks at boundaries
  and strict code would not; the tests must cover both until one remains.

## Decisions so far

Taken on 2026-11-22 and recorded in stage 0's decision records:

- **No dynamic source type.** Unannotated code is the dynamic part during
  the gradual phase; under strict typing, unknown data is `any|nil`,
  narrowed before use.
- **An annotation is accurate and complete.** An annotated place whose
  inferred type is wider than the annotation is a source error; only parts
  coming from unannotated code are checked at run time.
- **Record types have no optional fields.** `field: T|nil` covers absence,
  as for any nilable value, and only string keys name fields.
- **Enum payloads are braced named fields,** `Ok { value: T }`, consistent
  with struct construction. Positional payloads are revisited after a clutch
  is typed with `Result`.
- **Recursive aliases** are allowed when contractive and regular; nominal
  structs and enums may always refer to themselves.

## Open questions

- What annotation density makes strict typing acceptable, and how is it
  measured fairly?
