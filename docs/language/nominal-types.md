# Nominal types

Slug has three top-level declarations that introduce type names:

```slug
export resource File

export enum SeekFrom {
  Start
  Current
  End
}

export type Path = str
```

`resource` and `enum` introduce nominal types. `type` introduces a transparent
alias. These declarations are valid only at module top level. `export` makes a
type name available to importers in the module's compile-time type namespace;
ordinary values and type names remain separate namespaces. An exported resource
name is absent from an imported module's runtime value map.

## Resource handles

`resource File` declares an opaque, nominal native handle type. Slug has no
literal, constructor, field access, cast, serialization, or structural
destructuring operation for a resource handle. Only a native callback owned by
the declaring module can create one.

Each declared resource name corresponds to exactly one native resource type
registered by the same module under that exact name. A foreign call crossing
that boundary MUST validate declared resource arguments and results against
that registration.
The validation applies even when a call is dynamically reached or optional
static checking is disabled.

Hosts may inspect a module's declared resource names through its loaded module
metadata. This does not expose a Slug value, constructor, or reflection API for
the resource type.

Resource handles may occur in every ordinary type position, including unions,
collections, channels, function signatures, and whole-case type constraints:

```slug
export foreign openRead = fn(path:str):File
export foreign readLine = fn(file:File):str|nil

val files:list<File> = []
val pending:chan<File> = chan(8)
```

Resource types are distinct from every other type, including another resource
type declared in the same module. There is no broad `resource` source type:
an API accepting more than one handle kind MUST state a union of its accepted
nominal types. A resource's open state is dynamic; using a correctly typed
handle after `close` remains a checked runtime error.

## Enumerations

An enum declares a closed, nominal type and one immutable, fieldless value for
each case:

```slug
export enum SeekFrom {
  Start
  Current
  End
}

val from:SeekFrom = SeekFrom.Start
```

Cases are separated by a comma, one or more newlines, or both, and a trailing
comma is allowed, so `enum Colour { red, green, blue }` and one case per line
are equivalent. Two cases with no separator between them, such as
`enum Colour { red green }`, are a parse error: `expected comma or newline
between enum cases`. Semicolons and consecutive commas do not separate cases.

Cases are always qualified. `SeekFrom.Start` is a valid enum-value expression;
bare `Start` is an unknown name. The same rule applies to match patterns:

```slug
match from {
  SeekFrom.Start => 0
  SeekFrom.Current => 1
  SeekFrom.End => 2
}
```

Enum values compare equal only when both their enum identity and case are the
same. They are not strings or numbers, cannot be forged by casts, and do not
support explicit discriminants, bit flags, or methods. An enum's complete case
set is available to closed-match coverage checking.

### Payload cases

A case may carry a payload of named fields, in braces after its name on the
same line, and an enum may take type parameters, which its payloads' field
annotations name. The rationale is in
[Payload-bearing and generic enum variants](../decisions/2026-11-22-payload-enum-variants.md).

```slug
enum Result<T, E> {
  Ok { value: T }
  Error { error: E }
}

enum Shape { Circle { radius: num }, Square { side: num, label: str = "sq" }, Empty }

val parsed:Result<num, str> = Result.Ok { value: 3 }

val unwrapOr = fn<T>(r:Result<T, str>, fallback:T):T {
  match r {
    {value}: Result.Ok => value
    _: Result.Error => fallback
  }
}
```

- **Schemas.** A payload case is a struct schema, with the field rules of
  [Structs](structs.md) for types, defaults, and required fields; its
  defaults are evaluated once, when the enum is declared. `Result.Ok` is that
  schema, so `Result.Ok { value: 3 }` constructs a struct of it, and field
  reads, `copy`, equality, and record-type conformance are a struct's. An
  enum may mix fieldless and payload cases.
- **Types.** A value of a payload case is a value of its enum: `Result.Ok {
  value: 3 }` conforms to `Result<num, str>`. `Result<T, E>` takes one type
  argument per parameter, covariantly, as a generic schema's struct does;
  bare `Result` leaves them unknown. Construction infers a case's type
  arguments from its fields, and diagnostics show its type as the case,
  `Result.Ok<num, unknown>`, where a type argument no field determines is
  unknown. `Result.Ok` is also a type annotation, for that case's values.
- **Patterns.** A case is matched by a whole-pattern type constraint,
  `{value}: Result.Ok` or `_: Result.Error`. The pattern's fields take their
  types from the subject's type arguments, so `value` is a `num` when the
  subject is a `Result<num, str>`. There is no constructor-shaped pattern,
  such as `Result.Ok { value }`.
- **Coverage.** A closed match covers a payload case with a constraint
  naming it whose pattern matches every struct of the case: a wildcard, a
  binding, or a map pattern of its fields whose patterns are irrefutable.
  `{radius: 1}: Shape.Circle` matches some circles and covers none. An
  unhandled case makes a closed match non-exhaustive, and a repeated one is
  unreachable.
- **`Result`.** `slug.std` declares `enum Result<T, E> { Ok { value: T }
  Error { error: E } }` for operations that can fail, with `or`, `orElse`,
  and `orThrow` to take the value out, best through the pipeline:
  `read(path) /> or(defaults)`. `val {Result} = import("slug.std")` brings
  the type with the value, as naming any imported type does.
- **At run time** a payload case's value is a struct of its case's schema,
  which prints as a struct does. Type arguments are erased: a match
  constraint names a generic enum as `Result`, which matches its fieldless
  values and a struct of any of its payload cases; `Result<num, str>` is not
  runtime-checkable.

## Aliases

`type Name = Annotation` gives an existing type another name:

```slug
export type Path = str
export type MaybeFile = File|nil
```

Aliases are transparent. `Path` and `str` are interchangeable for
assignability, overload selection, callable-signature identity, and runtime
matching. An alias does not create a value, a constructor, or a new nominal
identity. Strong typedefs are outside this feature.

A generic alias takes type parameters, which each use fills in:

```slug
type Optional<T> = T|nil
type Pair<T, U> = [T, U]
type Twin<T> = Pair<T, T>

val label = fn(name:Optional<str>):str { name ?? "anon" }
val twins:Twin<num> = [1, 2]
```

`Optional<str>` stands for `str|nil` wherever it is written, including in
another alias, a generic function's signature, and a match type constraint,
where the expanded type must be runtime-checkable. Its parameters are written
once each, after the alias name. A use must give exactly as many type
arguments as the alias has parameters: `Optional` alone, `Pair<num>`, and
`Path<str>` for an alias `Path` without parameters are source errors, with
code `invalid_type_arguments`. An imported generic alias is used the same way,
as `shapes.Optional<num>`, and keeps the struct types of its module.

An alias may refer to itself, directly or through other aliases, for trees
and nested data:

```slug
type Json = nil|bool|num|str|list<Json>|map<str, Json>
type Tree<T> = [T, list<Tree<T>>]
type Even = list<Odd>|nil
type Odd = map<str, Even>
```

A recursive alias must be contractive and regular; otherwise it is a source
error with code `recursive_type_alias`:

- **Contractive.** Every reference back to an alias being defined is inside
  `list`, `map`, `task`, `chan`, a tuple, or a function type, so that the
  alias describes values. `type Loop = Loop|nil` and `type A = B` with
  `type B = A` are errors, and so is a reference through another alias's
  parameter that is not inside one: with `type Opt<T> = T|nil`, `type L =
  Opt<L>` is an error, while with `type Box<T> = list<T>`, `type L =
  Box<L>|nil` is allowed.
- **Regular.** A generic alias refers back to itself with the same type
  arguments it was given: `type Tree<T> = [T, list<Tree<T>>]` is allowed,
  `type Nest<T> = list<Nest<list<T>>>` is an error.

Recursive types are compared by their structure, assuming that a comparison
met again holds, and diagnostics name them by their alias, as `expected Json,
got list<num|str>`. A recursive type is not yet runtime-checkable, so it cannot
be a match type constraint.

An alias of a struct type names the schema where the alias is declared, so a
module can export a struct type while keeping its schema private:

```slug
val PointSchema = struct { x: num, y: num }
export type Point = struct<PointSchema>
export val point = fn(x:num, y:num):Point { PointSchema {x: x, y: y} }
```

An importer writes `shapes.Point`, and its field accesses are checked against
`PointSchema`, though it cannot name or construct the schema.

## Imports

An exported type is selected through the compile-time type namespace of an
imported module binding. Given a module binding named `fs`, `fs.File` names the
exported `File` type:

```slug
val fs = import("slug.io.fs")
val file:fs.File = fs.openRead("records.csv")
```

The identifier before the first `.` in such a type path MUST resolve to a
module binding created by `import`. The remaining path selects an exported type
from that module. A type path preserves its declaring module and declaration
name as nominal identity; importing, renaming, or aliasing a type MUST NOT make
two independently declared resource or enum types equal.

The top-level import selector `val {*} = import("module")` additionally brings
that module's exported type names into the local type namespace, alongside its
ordinary value exports. It does not create runtime values for resource types.

Enum declarations also expose their enum namespace as an ordinary immutable
value. Importing an enum therefore preserves qualified case access:

```slug
val options = import("slug.io.options")
val from:options.SeekFrom = options.SeekFrom.Start
```

Importing an enum does not inject its case names into the importing module.

## Implementation status

The current Rust subset implements nominal resource declarations, exported
resource type paths such as `fs.File`, the `File` signatures in `slug.io.fs`,
checked source/native resource-registration agreement, mandatory foreign
resource argument/result validation, and enum declarations, fieldless and
payload-bearing, generic or not, with qualified local/imported values and
patterns, type paths, and exhaustive closed-match coverage, and transparent
aliases including exported/imported
and generic aliases. Aliases are compile-time-only names: they have no value, constructor,
display form, or equality behavior of their own.
