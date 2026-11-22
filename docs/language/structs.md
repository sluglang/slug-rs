# Structs

This supplement defines schema values, basic struct construction, field access,
equality, copying, and patterns. Field type annotations remain a separate
implementation stage.

## Schemas and defaults

A struct expression creates a new schema identity every time it is evaluated.
Its value has type `schema`; values constructed through a direct known schema
binding `S` have type `struct<S>`. `schema` itself has no type arguments.
Fields are ordered by their declaration order and names must be unique. A field
without a default is required. Schema fields are separated by a comma, one or
more newlines, or both, and a trailing comma is allowed. Two fields with no
separator between them, such as `x: num y: num`, are a parse error: `expected
comma or newline between struct fields`. Semicolons and consecutive commas do
not separate fields.

    val User = struct {
      name,
      active = true,
    }

Default expressions evaluate once, in source order, when the schema expression
is evaluated. They use the schema expression's lexical environment. Constructed
values reuse the resulting default values; construction does not reevaluate
default expressions.

The current Rust subset accepts fields with optional annotations and defaults.
During compilation, a statically known default must conform to its field
annotation. An unannotated field's type is set by its default, as a `var`'s is
by its initializer: `count = 0` accepts only numbers, while a `nil` default,
`error = nil`, declares `any|nil`, so construction and copying may give the
field any value. A field with neither has no static type. Known local, aliased, and imported schema bindings retain their
field metadata for construction, copy, and direct field-access checks;
annotations do not coerce runtime values.

In a `struct<S>` annotation, `S` must be a lexical schema binding. The
annotation records that schema's stable identity, so an alias has the original
schema's nominal type and later shadowing cannot change field checking for an
existing struct value.

## Generic schemas

A schema may take type parameters, written after `struct` as a generic
function's are after `fn`. Its field annotations name them:

    val Pair = struct<T, U> { first: T, second: U }
    val Box = struct<T> {
      value: T
      tags: list<str> = []
    }

A struct of a generic schema `S` has type `struct<S<A, ...>>`, with one type
argument per parameter; `struct<S>` leaves them unknown, as `list` leaves its
element type. Construction infers each type argument from the fields given,
as a generic call infers its type arguments from its arguments, so
`Pair { first: 1, second: "a" }` has type `struct<Pair<num, str>>`. A type
argument no given field determines is unknown, and one cannot include `nil`.
Field reads, destructuring, and patterns see the field types with the type
arguments filled in: `p.first` is a `num`. A copy keeps its value's type, so
each replacement must conform to its field's type with the value's type
arguments. A default cannot have a type parameter's type, since it is one
value for every struct; `tags: list<str> = []` is a default, `value: T = 1`
is a source error.

Type arguments are covariant, as fields are immutable:
`struct<Pair<num, str>>` conforms to `struct<Pair<num, str|nil>>`. A wrong
number of type arguments, or any for a schema that takes none, is a source
error. A generic schema's fields may name only its own type parameters, not
those of a generic function around it.

Type arguments are erased: a struct's value records its schema, not them.
So a match type constraint may name a generic schema only as `struct<S>`;
`struct<Box<num>>` is not runtime-checkable and is a source error.

## Construction

Applying a schema to a brace-delimited field list constructs a value:

    val user = User {name: "Slug"}

Construction and copy fields require commas, with an optional trailing comma;
newlines may follow a comma or precede the closing brace. A missing comma is a
parse error: `expected comma between struct fields`.

The schema expression is evaluated first, followed by provided field expressions
in source order. Construction fails through the checked runtime type-error path
when the target is not a schema, a field is unknown, a field is provided more
than once, or a required field is omitted. Omitted fields with defaults receive
their schema's stored default values.

During compilation, construction through a known schema also rejects duplicate
or unknown fields, missing required fields, and supplied values that do not
conform to declared or inferred field types. Dynamically selected schemas keep
the ordinary runtime behavior.

An empty construction used directly as a match subject is parenthesized to
distinguish its empty field list from the match case block:

    match (Marker {}) { _ => true }

## Access and equality

Dot access and string-key bracket access read a struct field. Accessing an
unknown field or using a non-string struct index follows the checked runtime
type-error path.

Each schema compares equal only to itself. Two struct values compare equal when
they have the same schema identity and their field values compare equal in
schema order. Values created from distinct schema evaluations are unequal even
when their field names and values are otherwise identical.

## Copying

`value copy { field: replacement }` creates a new struct value with the same
schema identity as `value`. It evaluates `value` first and replacement
expressions left to right. Each named field is replaced; fields not named in
the copy retain their original values. This form also copies maps; see
[Maps](maps.md) for map-key behavior. Copying a value that is neither a struct
nor a map, naming an unknown struct field, or naming a field more than once is
a checked runtime type error.
During compilation, a known `struct<S>` additionally checks replacement value
types and infers direct field reads from `S`'s field metadata.

## Patterns

Struct fields use ordinary map-pattern syntax together with a type constraint.
For example, `user @ {name, active: true}: struct<User>` matches a value only
when it has the exact schema identity denoted by `User` and its named fields
match. Field requirements are partial: omitted fields are ignored, and a
shorthand field such as `name` binds that field to `name`. Duplicate field names
are invalid source.

During compilation, a field bound from a known `struct<User>` has the type
declared for that field by `User`, including in a destructuring declaration.

`_: struct` matches every struct value. The `User` binding in `struct<User>`
must resolve to a schema; a non-schema binding follows the checked runtime
type-error path. See [Match and Destructuring](match-and-destructuring.md)
for constraint evaluation and the current implementation boundary.
