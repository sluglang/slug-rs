# Annotation Checks

An annotation is a guarantee: a value held in an annotated place has the
annotated type. The checker proves this where it can, and a runtime check
verifies it where the value comes from code the checker could not type. The
rationale is in [Annotations are guarantees](../decisions/2026-11-22-annotations-are-guarantees.md)
and [the transient soundness model](../decisions/2026-11-22-transient-soundness-model.md).

Status: implemented for declarations, assignments, parameters, results,
`return`, and a recovering `defer onerror`, struct fields, calls, and
`foreign` results, with dynamic overload dispatch by type; [stage 6 of the
typed-Slug plan](../planning/typed-slug.md#6-sound-boundaries) has the
details.

```slug
val parse = fn(text) { text }        // unannotated: its result is not known
val port:num = parse("80")           // checked here: "80" is a str

// runtime error: expected num, got str: the value of `port`
```

## The static rule

The type the checker infers for what an annotation describes must conform to
it, member by member. A body that may return `num|str` under `:num`, or an
initializer of type `str|nil` for a binding annotated `str`, is a source
error. Only the part of an inferred type that the checker could not
determine, the dynamic part from unannotated code, is left to a runtime check.

## Proven and dynamic values

A value's type is **proven** when the checker determined it completely and
the runtime enforces it. It is **dynamic**, in whole or part, when the checker
could not determine it: an unannotated parameter with no inferred type, the
result of a call the checker could not resolve, an element of a list or map
whose element type is not known, and values derived from these.

The checker's type for a value is proven, with these exceptions, where the
runtime checks it at the value's source instead:

| Source | Why the type is not enforced already | Checked |
| --- | --- | --- |
| A call through a function value, such as a parameter `f:fn<bool, T>` | The value was only checked to be a function | The result, at the call, against the function type's result |
| A receive from a `chan<T>`, or a `select` receive case | The channel was only checked to be a channel | The received value, against `T` |
| An `await` of a `task<T>` | The task was only checked to be a task | The task's result, against `T` |
| A call to a `foreign` function with a result annotation | Host code is not typed | The result, when the host function returns |
| A generic call whose callee may return, at a type parameter, a value it could not prove | A type parameter's check is erased, so the callee cannot check it | The result, at the call, against the type the call instantiates |
| A generic call with an argument that is, or holds, a function value other than a function literal or a declared function | The value was only checked to be a function, and a type argument may come from its type | The result, at the call, against the type the call instantiates |

A function type's result, `T`, or a payload, that is a type parameter or
`any|nil` is not checked: nothing relies on it.

So in `fn<T>(x:T):T { helper(x) }`, with `helper` unannotated, the function
cannot check what it returns, and `val n:num = launder(1)` checks `n`'s
value at the call. And `map(xs, g)`, where `g` is a parameter of type
`fn<num, str>`, checks that its result is a `list<num>`, since nothing made
`g` return numbers; `map(xs, fn(s) { len(s) })` checks nothing there, as the
literal's result is proven.

A builtin's result is proven: the runtime provides the builtins and their
signatures together.

## Where checks occur

A check is inserted where a value whose type is not proven reaches an
annotated place, and only there. Where the checker proved the type, no check
is made.

| Place | Checked | When |
| --- | --- | --- |
| `val name:T = value`, and a destructuring declaration with an annotation | `value` against `T` | Before the binding is made |
| `name = value`, for a `var` of type `T` | `value` against `T` | Before the assignment |
| A parameter of type `T` | The argument, or the default, against `T` | On entry to the function, before its body runs |
| A rest parameter `...name:list<T>` | The list of rest arguments | On entry |
| A function's result annotation `:T` | The value it completes with, by reaching the end of its body or by `return` | When it returns |
| A struct field `name:T` | The field's value against `T`, with the struct's type arguments | At construction, and at a `copy` that replaces the field |
| A field of a struct whose schema the checker does not know, as `s { name: v }` or `p copy { name: v }` for unannotated `s` and `p` | The value's kind against the schema's annotation for the field, which the schema carries | At construction or the copy, by the schema itself |
| The sources in the table above | As described there | At the source |

**What the checker relied on is checked, annotated or not.** A `var`'s type
is its annotation, or else the type of its first value, which every
assignment must conform to. A parameter's type is its annotation, or else
the type the checker inferred inside the body: from how the body uses it, as
`v` is a `num` in `fn(v) { v / 10 }`, or from what a callee expects of an
inline function. The checker relies on those types as it does on
annotations, so they are checked in the same places. A parameter whose type
is one of several body-derived alternatives, as for `fn(a, b) { a + b }`, is
not checked: the body is checked against each alternative.

A `recur` that passes every argument positionally re-enters its function
past the entry checks: its arguments are checked by the checker against the
parameters, and any it cannot prove are checked at the `recur`. So a
recursive helper's `list<num>` accumulator is not scanned again on each
iteration. A `recur` with named or spread arguments restarts at the entry,
checks included.

**Parameters are checked by the function, not its callers.** A function
cannot know whether its caller was typed, since any function may be passed
as a value. Inside a function, its parameters' types are proven.

A call that names its function directly, as `render(nodes, out)` names a
`val`, and gives every parameter positionally does as a full `recur` does:
the checker checks its arguments against the parameters, the call checks
where it is any argument the checker cannot prove, and it enters past the
function's entry checks. A proven `list<str>` accumulator passed from one
helper to another is then not scanned again on each call. It is an
optimization, not a change of meaning: an unproven argument fails as the
entry check would, naming the same parameter. A call through a function
value, of an overload set, of a generic function, or with named, spread, or
defaulted arguments enters at the start, checks included.

**A function literal given to a parametric function checks nothing on
entry.** A generic function is parametric when it calls each of its
function-typed parameters only with exactly the types they take, lets none
of them escape, being only called, piped into, or given to its own `recur`,
and proves its result. Such a function passes a function literal only
values that came from its own arguments or from the literal's results. So a
literal given to it, when the other arguments are proven and the literal's
result is too, as `reduce(xs, state, fn(s, x) { step(s, x) })` for a proven
`state`, has no entry checks: its parameters are proven by the call.
`slug.std`'s `map`, `filter`, `reduce`, and `each` are parametric.

**A dynamic call to an overload set selects by type.** A call the checker
could not resolve, as one through an overload set passed as a value, takes
the first member whose parameters bind the arguments and whose parameters'
kinds they have, so `apply(f, "s")` reaches `f(x:str)` though `f(x:num)`
was declared first. A call that no member accepts reaches the first member
that binds the arguments, whose entry check fails naming it.

**A `foreign` result and a schema's fields are checked by kind.** Host
code gives a `foreign` function's result, which is checked to be of the
annotation's kind when it returns: a `list<str>` result is checked to be a
list. A schema carries the kinds of its annotated fields, and checks every
value given them, by a construction or a copy, wherever it happens.

## What a check covers

A check is the same test a match type constraint makes, extended where match
constraints cannot yet go.

- **First-order types:** `nil`, `bool`, `num`, `str`, `bytes`, `schema`, a
  resource type, an enum, a payload case, and `struct<S>` test the value's
  tag or nominal identity. `any` tests that the value is not `nil`. A union
  tests that the value satisfies a member.
- **Immutable values:** `list<T>`, `map<K, V>`, a tuple type, a record type,
  and a struct or enum value with type arguments, as `Box<num>` or
  `Result<num, str>`, are checked deeply: each element, entry, or field
  against its type. Since the value cannot change, it is checked once for a
  type: the result is kept with the value, so a later check of the same value
  against the same type costs no scan. A collection built in typed code from
  values of proven types is checked by construction.
- **Functions, tasks, and channels:** the value's tag is checked, as
  `function`, `task`, or `channel`. What it returns, completes with, or
  carries is checked at the sources above.
- **Type parameters** are not checked. In `fn<T>(x:T):T`, `T` checks
  nothing; at a call, the type arguments the checker inferred are checked
  where the value reaches a concrete type.
- **Aliases** are checked as what they stand for; a recursive alias is
  checked by unfolding it as the value is walked, which ends, as a value is
  finite.
- **Tuples** are checked by position: `[num, str]` is a list of two, a
  number then a string.
- **A generic struct or enum** is checked for its schema or cases and, with
  its type arguments, for each field whose declared type uses a type
  parameter: `struct<Box<num>>` checks that `value` is a number, and
  `Result<num, str>` that an `Ok`'s `value` is.

## The error

A failed check is a checked runtime error, with the code
`annotation_mismatch`, raised where the value reaches the place. It unwinds
as any runtime fault does, so `defer onerror` sees it, with its `type` field
`"annotation_mismatch"`. It reads `expected T, got U: place`, where `U` is
the shallow type of the value, as `str`, `list`, or an enum's name, and the
place is one of:

| Place | Named as |
| --- | --- |
| A declaration | the value of `name` |
| An assignment | the value assigned to `name` |
| A parameter | the argument `name`, with "whose type the function's body relies on" for an inferred type |
| A result or `return` | the function's result |
| A struct field | the field `name` |
| A call's result | the call's result |
| A `foreign` result | the result of foreign `module.name` |

A deep check adds the first part of the value that failed, as a path from
it, with that part's type and the type it must have:

```text
runtime error: expected list<{name: str}>, got list: the argument `people`: the part `[1].name` is num, but must be str
  --> lib.slug:4:13
  in <fn #1> at main.slug:9:12
```

Its location is the declaration, assignment, `return`, struct, or call; for
a parameter or a body's result, the function whose signature has the
annotation, with the call in the frames. A field a schema checks where the
struct is made names where the schema, which has the annotation, was
declared: `the field \`name\`, annotated in the schema at pets.slug:1:17`.

## Not checked

- An unannotated place. Unannotated code is the dynamic part of a program:
  a value there is checked only when it reaches an annotation.
- A map's values or a list's elements in an unannotated place, until the
  collection reaches an annotated one.
- What a function value will be called with: a function's parameter types
  are guaranteed only inside it, by its entry checks.
