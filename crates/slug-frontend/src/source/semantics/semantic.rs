use std::{
    collections::HashMap,
    fmt,
    hash::{Hash, Hasher},
};

use slug_vm::SourceSpan;

use super::{SourceError, SourceErrorCode, ast::TypeAnnotation, environment::Environment};

/// Private identity shared by nominal source declarations.
///
/// `declaration_path` and `declaration_name` identify the declaration; `name`
/// is the spelling visible at the current use site and may therefore differ
/// after an import.
#[derive(Clone, Debug)]
pub(super) struct NominalIdentity {
    declaration_path: String,
    declaration_name: String,
    pub(super) name: String,
    runtime_module: Option<String>,
}

impl NominalIdentity {
    pub(super) fn declared(path: impl Into<String>, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            declaration_path: path.into(),
            declaration_name: name.clone(),
            name,
            runtime_module: None,
        }
    }

    fn unresolved(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            declaration_path: String::new(),
            declaration_name: name.clone(),
            name,
            runtime_module: None,
        }
    }

    pub(super) fn is_unresolved(&self) -> bool {
        self.declaration_path.is_empty()
    }

    pub(super) fn runtime_module(&self) -> &str {
        self.runtime_module
            .as_deref()
            .unwrap_or(&self.declaration_path)
    }

    pub(super) fn explicit_runtime_module(&self) -> Option<&str> {
        self.runtime_module.as_deref()
    }

    pub(super) fn set_runtime_module(&mut self, module: String) {
        self.runtime_module = Some(module);
    }

    fn diagnostic_name(&self) -> String {
        let module = std::path::Path::new(&self.declaration_path)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or(&self.declaration_path);
        format!("{module}.{}", self.name)
    }
}

impl PartialEq for NominalIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.declaration_path == other.declaration_path
            && self.declaration_name == other.declaration_name
    }
}

impl Eq for NominalIdentity {}

impl Hash for NominalIdentity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.declaration_path.hash(state);
        self.declaration_name.hash(state);
    }
}

pub(super) type SchemaIdentity = NominalIdentity;
pub(super) type ResourceIdentity = NominalIdentity;
pub(super) type EnumIdentity = NominalIdentity;

#[derive(Debug, Eq, PartialEq)]
pub(super) enum Type {
    /// Internal bottom type for expressions that cannot produce a value.
    Never,
    Unknown,
    Any,
    Nil,
    Bool,
    Num,
    Str,
    Bytes,
    Resource(ResourceIdentity),
    Enum(EnumIdentity),
    List(Option<Box<Type>>),
    Map(Option<(Box<Type>, Box<Type>)>),
    Function(Option<Vec<Type>>),
    Task(Option<Box<Type>>),
    Channel(Option<Box<Type>>),
    Schema,
    /// A struct of the schema the identity names, or of any schema; for a
    /// generic schema, with its type arguments, empty when they are unknown.
    Struct(Option<SchemaIdentity>, Vec<Type>),
    Tuple(Vec<Type>),
    /// A record type, as `{name: str}`: a map whose value at each field's
    /// string key, or a struct whose field of that name, conforms to the
    /// field's type, whatever other keys or fields it has. Fields are sorted
    /// by name; build one with [`Type::record`]. With `map`, the type of a
    /// map literal: a map whose keys are exactly these strings.
    Record {
        fields: Vec<(String, Type)>,
        map: bool,
    },
    Generic(usize),
    Union(Vec<Type>),
    /// A generic alias applied to type arguments, as `Optional<num>`, until
    /// annotation resolution looks the alias up and fills in its body.
    AliasApplication(String, Vec<Type>),
    /// A recursive alias's type: its body, in which `RecursionVar(name)`
    /// stands for the whole type again. It carries its own definition, so it
    /// unfolds without the environment.
    Recursive {
        name: String,
        body: Box<Type>,
    },
    /// Inside a `Recursive` body named `name`, the recursive type itself.
    RecursionVar(String),
}

/// A schema's type parameter count and field types.
type SchemaFields = (usize, HashMap<String, Type>);

std::thread_local! {
    /// The field types of each schema the checker has seen, so a struct type,
    /// which carries only its schema's identity, can be compared with a
    /// record type.
    /// A generic schema's fields refer to its type parameters as
    /// `Type::Generic(i)`; the count is kept with them.
    static SCHEMA_FIELDS: std::cell::RefCell<HashMap<SchemaIdentity, SchemaFields>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Records the field types of the schema `identity` names, for comparing its
/// structs with record types.
pub(super) fn register_schema_fields(
    identity: &SchemaIdentity,
    arity: usize,
    fields: HashMap<String, Type>,
) {
    SCHEMA_FIELDS.with(|schemas| {
        schemas
            .borrow_mut()
            .insert(identity.clone(), (arity, fields));
    });
}

/// The field types of a struct of the schema `identity` names, with
/// `arguments` for a generic schema's type parameters, when the checker has
/// seen it.
pub(super) fn schema_fields(
    identity: &SchemaIdentity,
    arguments: &[Type],
) -> Option<HashMap<String, Type>> {
    let (arity, fields) = SCHEMA_FIELDS.with(|schemas| schemas.borrow().get(identity).cloned())?;
    Some(instantiate_fields(arity, fields, arguments))
}

/// A generic schema's `fields` with `arguments` for its `arity` type
/// parameters, each unknown when it is not given.
pub(super) fn instantiate_fields<K: Eq + Hash>(
    arity: usize,
    fields: HashMap<K, Type>,
    arguments: &[Type],
) -> HashMap<K, Type> {
    if arity == 0 {
        return fields;
    }
    let substitutions = arguments.iter().cloned().enumerate().collect();
    fields
        .into_iter()
        .map(|(name, field)| (name, substitute(&field, &substitutions)))
        .collect()
}

/// Types nest as deeply as source annotations may, so a clone, which goes
/// one level down per call, runs through [`super::stack::grow`].
impl Clone for Type {
    fn clone(&self) -> Self {
        super::stack::grow(|| match self {
            Self::Never => Self::Never,
            Self::Unknown => Self::Unknown,
            Self::Any => Self::Any,
            Self::Nil => Self::Nil,
            Self::Bool => Self::Bool,
            Self::Num => Self::Num,
            Self::Str => Self::Str,
            Self::Bytes => Self::Bytes,
            Self::Resource(identity) => Self::Resource(identity.clone()),
            Self::Enum(identity) => Self::Enum(identity.clone()),
            Self::List(element) => Self::List(element.clone()),
            Self::Map(entries) => Self::Map(entries.clone()),
            Self::Function(signature) => Self::Function(signature.clone()),
            Self::Task(result) => Self::Task(result.clone()),
            Self::Channel(value) => Self::Channel(value.clone()),
            Self::Schema => Self::Schema,
            Self::Struct(identity, arguments) => Self::Struct(identity.clone(), arguments.clone()),
            Self::Tuple(elements) => Self::Tuple(elements.clone()),
            Self::Record { fields, map } => Self::Record {
                fields: fields.clone(),
                map: *map,
            },
            Self::Generic(index) => Self::Generic(*index),
            Self::Union(members) => Self::Union(members.clone()),
            Self::AliasApplication(name, arguments) => {
                Self::AliasApplication(name.clone(), arguments.clone())
            }
            Self::Recursive { name, body } => Self::Recursive {
                name: name.clone(),
                body: body.clone(),
            },
            Self::RecursionVar(name) => Self::RecursionVar(name.clone()),
        })
    }
}

std::thread_local! {
    /// Pairs of types being compared while one of them is recursive: a pair
    /// met again is assumed to conform, as recursive types are compared
    /// coinductively.
    static ASSUMED: std::cell::RefCell<Vec<(Type, Type)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// `body` with each free `RecursionVar(name)` replaced by `replacement`: a
/// nested recursive type of the same name binds its own.
fn replace_recursion(body: &Type, name: &str, replacement: &Type) -> Type {
    let recur = |value: &Type| replace_recursion(value, name, replacement);
    match body {
        Type::RecursionVar(var) if var == name => replacement.clone(),
        Type::Recursive { name: inner, .. } if inner == name => body.clone(),
        Type::Recursive { name: inner, body } => Type::Recursive {
            name: inner.clone(),
            body: Box::new(recur(body)),
        },
        Type::List(element) => {
            Type::List(element.as_deref().map(|element| Box::new(recur(element))))
        }
        Type::Map(entries) => Type::Map(
            entries
                .as_ref()
                .map(|(key, value)| (Box::new(recur(key)), Box::new(recur(value)))),
        ),
        Type::Function(signature) => Type::Function(
            signature
                .as_ref()
                .map(|types| types.iter().map(recur).collect()),
        ),
        Type::Task(result) => Type::Task(result.as_deref().map(|result| Box::new(recur(result)))),
        Type::Channel(value) => Type::Channel(value.as_deref().map(|value| Box::new(recur(value)))),
        Type::Tuple(elements) => Type::Tuple(elements.iter().map(recur).collect()),
        Type::Record { fields, map } => Type::Record {
            fields: fields
                .iter()
                .map(|(field, value)| (field.clone(), recur(value)))
                .collect(),
            map: *map,
        },
        Type::Struct(identity, arguments) => {
            Type::Struct(identity.clone(), arguments.iter().map(recur).collect())
        }
        Type::Union(members) => Type::Union(members.iter().map(recur).collect()),
        Type::AliasApplication(alias, arguments) => {
            Type::AliasApplication(alias.clone(), arguments.iter().map(recur).collect())
        }
        other => other.clone(),
    }
}

/// A variable whose meaning is scoped to one inferred overload alternative.
///
/// This deliberately does not reuse [`Type::Generic`]: source generic
/// parameters have callable-wide identity, while these variables describe a
/// single body-derived relationship.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[allow(dead_code)] // Wired into callable metadata by the following plan step.
pub(super) struct AlternativeVariable(usize);

#[allow(dead_code)]
impl AlternativeVariable {
    pub(super) fn new(index: usize) -> Self {
        Self(index)
    }
}

/// A type expression in a body-derived overload alternative.
///
/// The recursive forms are kept separate from [`Type`] so an alternative can
/// retain relationships such as `list<A> + list<B> -> list<A|B>` without
/// introducing private symbols into ordinary source types or their unions.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)] // The model precedes its first producer in this plan.
pub(super) enum AlternativeType {
    Concrete(Type),
    HashableMapKey,
    Variable(AlternativeVariable),
    List(Box<Self>),
    Map(Box<Self>, Box<Self>),
    Union(Vec<Self>),
}

#[allow(dead_code)]
impl AlternativeType {
    pub(super) fn concrete(value_type: Type) -> Self {
        Self::Concrete(value_type)
    }

    pub(super) fn variable(variable: AlternativeVariable) -> Self {
        Self::Variable(variable)
    }

    pub(super) fn hashable_map_key() -> Self {
        Self::HashableMapKey
    }

    pub(super) fn list(element: Self) -> Self {
        Self::List(Box::new(element))
    }

    pub(super) fn map(key: Self, value: Self) -> Self {
        Self::Map(Box::new(key), Box::new(value))
    }

    pub(super) fn union(members: impl IntoIterator<Item = Self>) -> Self {
        Self::normalized_union(members.into_iter().collect())
    }

    fn normalized_union(members: Vec<Self>) -> Self {
        let mut flattened = Vec::new();
        for member in members {
            match member {
                Self::Union(nested) => flattened.extend(nested),
                member => flattened.push(member),
            }
        }
        flattened.sort_by_key(ToString::to_string);
        flattened.dedup();
        match flattened.len() {
            0 => Self::Concrete(Type::Never),
            1 => flattened
                .pop()
                .expect("one normalized alternative union member"),
            _ => Self::Union(flattened),
        }
    }

    fn canonicalized(
        self,
        variables: &mut HashMap<AlternativeVariable, AlternativeVariable>,
        next_variable: &mut usize,
    ) -> Self {
        match self {
            Self::Concrete(value_type) => Self::Concrete(value_type),
            Self::HashableMapKey => Self::HashableMapKey,
            Self::Variable(variable) => {
                let variable = *variables.entry(variable).or_insert_with(|| {
                    let canonical = AlternativeVariable(*next_variable);
                    *next_variable += 1;
                    canonical
                });
                Self::Variable(variable)
            }
            Self::List(element) => {
                Self::List(Box::new(element.canonicalized(variables, next_variable)))
            }
            Self::Map(key, value) => Self::Map(
                Box::new(key.canonicalized(variables, next_variable)),
                Box::new(value.canonicalized(variables, next_variable)),
            ),
            Self::Union(members) => Self::normalized_union(
                members
                    .into_iter()
                    .map(|member| member.canonicalized(variables, next_variable))
                    .collect(),
            ),
        }
    }

    fn substitute(&self, substitutions: &HashMap<AlternativeVariable, Type>) -> Type {
        match self {
            Self::Concrete(value_type) => value_type.clone(),
            Self::HashableMapKey => Type::union([Type::Bool, Type::Num, Type::Str, Type::Bytes]),
            Self::Variable(variable) => substitutions
                .get(variable)
                .cloned()
                .unwrap_or(Type::Unknown),
            Self::List(element) => Type::List(Some(Box::new(element.substitute(substitutions)))),
            Self::Map(key, value) => Type::Map(Some((
                Box::new(key.substitute(substitutions)),
                Box::new(value.substitute(substitutions)),
            ))),
            Self::Union(members) => Type::union(
                members
                    .iter()
                    .map(|member| member.substitute(substitutions)),
            ),
        }
    }

    fn accepts_constraint(
        &self,
        actual: &Type,
        substitutions: &mut HashMap<AlternativeVariable, Type>,
    ) -> bool {
        if is_dynamic_alternative_constraint(actual) {
            return true;
        }
        match (self, actual) {
            (Self::Concrete(expected), actual) => actual.is_assignable_to(expected),
            (Self::HashableMapKey, actual) => is_hashable_map_key_type(actual),
            (Self::Variable(variable), actual) => {
                if let Some(previous) = substitutions.get(variable) {
                    actual == previous
                } else {
                    substitutions.insert(*variable, actual.clone());
                    true
                }
            }
            (Self::List(expected), Type::List(Some(actual))) => {
                expected.accepts_constraint(actual, substitutions)
            }
            (Self::Map(_, _), Type::Record { map: true, .. }) => {
                self.accepts_constraint(&actual.as_map(), substitutions)
            }
            (Self::List(_), Type::List(None)) | (Self::Map(_, _), Type::Map(None)) => true,
            (
                Self::Map(expected_key, expected_value),
                Type::Map(Some((actual_key, actual_value))),
            ) => {
                expected_key.accepts_constraint(actual_key, substitutions)
                    && expected_value.accepts_constraint(actual_value, substitutions)
            }
            (Self::Union(expected), actual) => expected.iter().any(|member| {
                let mut trial = substitutions.clone();
                if member.accepts_constraint(actual, &mut trial) {
                    *substitutions = trial;
                    true
                } else {
                    false
                }
            }),
            _ => false,
        }
    }
}

fn is_hashable_map_key_type(value_type: &Type) -> bool {
    match value_type {
        Type::Bool | Type::Num | Type::Str | Type::Bytes => true,
        Type::Union(members) => members.iter().all(is_hashable_map_key_type),
        _ => false,
    }
}

fn is_dynamic_alternative_constraint(value_type: &Type) -> bool {
    match value_type {
        Type::Unknown | Type::Any => true,
        Type::Union(members) => members.iter().any(is_dynamic_alternative_constraint),
        _ => false,
    }
}

impl fmt::Display for AlternativeType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Concrete(value_type) => formatter.write_str(&value_type.diagnostic_display()),
            Self::HashableMapKey => formatter.write_str("map-key"),
            Self::Variable(variable) => write!(formatter, "A{}", variable.0),
            Self::List(element) => write!(formatter, "list<{element}>"),
            Self::Map(key, value) => write!(formatter, "map<{key}, {value}>"),
            Self::Union(members) => write!(
                formatter,
                "{}",
                members
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("|")
            ),
        }
    }
}

/// A canonical parameter/result scheme inferred from one function body.
///
/// Its variables are local to this value. Constructing an alternative
/// canonicalizes variable identity by first occurrence, which makes equality
/// suitable for duplicate-signature identity without conflating it with
/// ordinary [`Type`] equality.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)] // Published after body derivation is implemented.
pub(super) struct InferredAlternative {
    parameters: Vec<AlternativeType>,
    result: AlternativeType,
}

#[allow(dead_code)]
impl InferredAlternative {
    pub(super) fn new(parameters: Vec<AlternativeType>, result: AlternativeType) -> Self {
        let mut variables = HashMap::new();
        let mut next_variable = 0;
        let parameters = parameters
            .into_iter()
            .map(|parameter| parameter.canonicalized(&mut variables, &mut next_variable))
            .collect();
        let result = result.canonicalized(&mut variables, &mut next_variable);
        Self { parameters, result }
    }

    pub(super) fn substitute(
        &self,
        substitutions: &HashMap<AlternativeVariable, Type>,
    ) -> (Vec<Type>, Type) {
        (
            self.parameters
                .iter()
                .map(|parameter| parameter.substitute(substitutions))
                .collect(),
            self.result.substitute(substitutions),
        )
    }

    pub(super) fn parameter_types(&self) -> &[AlternativeType] {
        &self.parameters
    }

    pub(super) fn result_type(&self) -> &AlternativeType {
        &self.result
    }

    /// Whether the known body facts are compatible with this symbolic scheme.
    /// Missing facts deliberately remain dynamic and cannot remove a scheme.
    pub(super) fn accepts_constraints(&self, constraints: &[Option<&Type>]) -> bool {
        if self.parameters.len() != constraints.len() {
            return false;
        }
        let mut substitutions = HashMap::new();
        self.parameters
            .iter()
            .zip(constraints)
            .all(|(parameter, constraint)| {
                constraint.is_none_or(|constraint| {
                    parameter.accepts_constraint(constraint, &mut substitutions)
                })
            })
    }

    pub(super) fn instantiate(&self, actuals: &[Type]) -> Option<(Vec<Type>, Type)> {
        if self.parameters.len() != actuals.len() {
            return None;
        }
        let mut substitutions = HashMap::new();
        if !self
            .parameters
            .iter()
            .zip(actuals)
            .all(|(parameter, actual)| parameter.accepts_constraint(actual, &mut substitutions))
        {
            return None;
        }
        Some(self.substitute(&substitutions))
    }
}

impl fmt::Display for InferredAlternative {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "({}) -> {}",
            self.parameters
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            self.result
        )
    }
}

impl Type {
    pub(super) fn diagnostic_display(&self) -> String {
        match self {
            Self::Resource(identity) | Self::Enum(identity) => identity.diagnostic_name(),
            Self::Struct(Some(identity), arguments) if arguments.is_empty() => {
                format!("struct<{}>", identity.diagnostic_name())
            }
            Self::Struct(Some(identity), arguments) => format!(
                "struct<{}<{}>>",
                identity.diagnostic_name(),
                arguments
                    .iter()
                    .map(Self::diagnostic_display)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::List(Some(value)) => format!("list<{}>", value.diagnostic_display()),
            Self::Task(Some(value)) => format!("task<{}>", value.diagnostic_display()),
            Self::Channel(Some(value)) => format!("chan<{}>", value.diagnostic_display()),
            Self::Union(values) => values
                .iter()
                .map(Self::diagnostic_display)
                .collect::<Vec<_>>()
                .join("|"),
            _ => self.to_string(),
        }
    }

    pub(super) fn is_reifiable_match_constraint(&self) -> bool {
        match self {
            Self::Any
            | Self::Nil
            | Self::Bool
            | Self::Num
            | Self::Str
            | Self::Bytes
            | Self::Resource(_)
            | Self::Enum(_)
            | Self::Function(None)
            | Self::Task(None)
            | Self::Channel(None)
            | Self::Schema
            // An alias application is resolved by the environment, and checked
            // again once it is, as an unresolved alias name is.
            | Self::AliasApplication(..) => true,
            Self::List(element) => element
                .as_deref()
                .is_none_or(Self::is_reifiable_match_constraint),
            Self::Map(entries) => entries.as_ref().is_none_or(|(key, value)| {
                key.is_reifiable_match_constraint() && value.is_reifiable_match_constraint()
            }),
            Self::Union(members) => members.iter().all(Self::is_reifiable_match_constraint),
            // A struct's type arguments are erased, so only its schema is
            // checked.
            Self::Struct(_, arguments) => arguments.iter().all(|argument| *argument == Self::Unknown),
            Self::Record { fields, .. } => fields
                .iter()
                .all(|(_, field)| field.is_reifiable_match_constraint()),
            Self::Never
            | Self::Unknown
            | Self::Function(Some(_))
            | Self::Task(Some(_))
            | Self::Channel(Some(_))
            | Self::Tuple(_)
            | Self::Generic(_)
            // A recursive type needs a recursive runtime check, which match
            // constraints do not have yet.
            | Self::Recursive { .. }
            | Self::RecursionVar(_) => false,
        }
    }
    pub(super) fn universal() -> Self {
        Self::Union(vec![Self::Any, Self::Nil])
    }

    /// A record type of these fields, sorted by name.
    pub(super) fn record(mut fields: Vec<(String, Self)>, map: bool) -> Self {
        fields.sort_by(|(left, _), (right, _)| left.cmp(right));
        Self::Record { fields, map }
    }

    /// A map literal's record type as the map type it also is, of string keys
    /// and the union of its fields' types; any other type as it is.
    pub(super) fn as_map(&self) -> Self {
        match self {
            Self::Record { fields, map: true } => Self::Map(Some((
                Box::new(Self::Str),
                Box::new(Self::union(fields.iter().map(|(_, value)| value.clone()))),
            ))),
            other => other.clone(),
        }
    }

    /// The type of field `name` of a value of this record type, if it has
    /// that field.
    pub(super) fn record_field(&self, name: &str) -> Option<&Self> {
        let Self::Record { fields, .. } = self else {
            return None;
        };
        fields
            .binary_search_by(|(field, _)| field.as_str().cmp(name))
            .ok()
            .map(|index| &fields[index].1)
    }

    pub(super) fn union(members: impl IntoIterator<Item = Self>) -> Self {
        let mut flattened = Vec::new();
        for member in members {
            match member {
                Self::Union(nested) => flattened.extend(nested),
                member => flattened.push(member),
            }
        }
        flattened.retain(|member| !matches!(member, Self::Never));
        if flattened.iter().any(|member| member == &Self::Unknown) {
            return Self::Unknown;
        }
        // A type parameter may stand for a nilable type, so `any` does not
        // cover it, and nothing covers it until it is filled in.
        if flattened.iter().any(|member| member == &Self::Any) {
            flattened.retain(|member| matches!(member, Self::Any | Self::Nil | Self::Generic(_)));
        }
        flattened.sort_by_key(sort_key);
        flattened.dedup();
        // A member that another member already covers adds nothing, such as
        // `list<num>` beside `list<num|str>`. Of two members that cover each
        // other, the first is kept.
        let covered = (0..flattened.len())
            .filter(|&index| {
                let member = &flattened[index];
                !matches!(member, Self::Generic(_))
                    && flattened.iter().enumerate().any(|(other_index, other)| {
                        other_index != index
                            && member.is_assignable_to(other)
                            && (!other.is_assignable_to(member) || other_index < index)
                    })
            })
            .collect::<Vec<_>>();
        for index in covered.into_iter().rev() {
            flattened.remove(index);
        }
        match flattened.len() {
            0 => Self::Never,
            1 => flattened.pop().expect("one normalized union member"),
            _ => Self::Union(flattened),
        }
    }

    /// The type with a recursive type at its top unfolded once, as many times
    /// as it takes to show its structure; any other type as it is. A
    /// recursive type's body is contractive, so this ends.
    pub(super) fn unfolded(&self) -> Self {
        let mut value = self.clone();
        while let Self::Recursive { name, body } = &value {
            value = replace_recursion(body, name, &value);
        }
        value
    }

    pub(super) fn includes_nil(&self) -> bool {
        if let Self::Recursive { .. } = self {
            return self.unfolded().includes_nil();
        }
        match self {
            Self::Unknown | Self::Nil => true,
            Self::Union(members) => members.iter().any(Self::includes_nil),
            _ => false,
        }
    }

    pub(super) fn without_nil(&self) -> Self {
        if let Self::Recursive { .. } = self {
            return self.unfolded().without_nil();
        }
        match self {
            Self::Nil => Self::Unknown,
            Self::Union(members) => Self::union(
                members
                    .iter()
                    .filter(|member| !matches!(member, Self::Nil))
                    .cloned(),
            ),
            other => other.clone(),
        }
    }

    pub(super) fn widen_unknown(self) -> Self {
        match self {
            Self::Unknown => Self::universal(),
            Self::List(element) => {
                Self::List(element.map(|element| Box::new(element.widen_unknown())))
            }
            Self::Map(entries) => Self::Map(entries.map(|(key, value)| {
                (
                    Box::new(key.widen_unknown()),
                    Box::new(value.widen_unknown()),
                )
            })),
            Self::Function(signature) => Self::Function(
                signature.map(|types| types.into_iter().map(Self::widen_unknown).collect()),
            ),
            Self::Task(result) => Self::Task(result.map(|result| Box::new(result.widen_unknown()))),
            Self::Channel(value) => {
                Self::Channel(value.map(|value| Box::new(value.widen_unknown())))
            }
            Self::Tuple(elements) => {
                Self::Tuple(elements.into_iter().map(Self::widen_unknown).collect())
            }
            Self::Record { fields, map } => Self::Record {
                fields: fields
                    .into_iter()
                    .map(|(name, value)| (name, value.widen_unknown()))
                    .collect(),
                map,
            },
            Self::Union(members) => Self::union(members.into_iter().map(Self::widen_unknown)),
            other => other,
        }
    }

    /// A tuple as the list it also is, of the union of its elements; any
    /// other type as it is.
    pub(super) fn as_list(&self) -> Self {
        match self {
            Self::Tuple(elements) if elements.is_empty() => Self::List(None),
            Self::Tuple(elements) => Self::List(Some(Box::new(Self::union(elements.clone())))),
            other => other.clone(),
        }
    }

    /// The type with every tuple in it widened to a list, and every map
    /// literal's record type to its map type, for a `var`, whose later values
    /// need not keep its first value's length or keys.
    pub(super) fn widen_literals(&self) -> Self {
        match self {
            Self::Tuple(_) => self.as_list().widen_literals(),
            Self::Record { map: true, .. } => self.as_map().widen_literals(),
            Self::List(Some(element)) => Self::List(Some(Box::new(element.widen_literals()))),
            Self::Map(Some((key, value))) => Self::Map(Some((
                Box::new(key.widen_literals()),
                Box::new(value.widen_literals()),
            ))),
            Self::Union(members) => Self::union(members.iter().map(Self::widen_literals)),
            other => other.clone(),
        }
    }

    /// The type with every tuple in it widened to a list.
    pub(super) fn widen_tuples(&self) -> Self {
        match self {
            Self::Tuple(_) => self.as_list().widen_tuples(),
            Self::List(Some(element)) => Self::List(Some(Box::new(element.widen_tuples()))),
            Self::Map(Some((key, value))) => Self::Map(Some((
                Box::new(key.widen_tuples()),
                Box::new(value.widen_tuples()),
            ))),
            Self::Union(members) => Self::union(members.iter().map(Self::widen_tuples)),
            other => other.clone(),
        }
    }

    pub(super) fn is_assignable_to(&self, expected: &Self) -> bool {
        if self == expected || matches!(self, Self::Never | Self::Unknown) {
            return true;
        }
        if matches!(self, Self::Recursive { .. }) || matches!(expected, Self::Recursive { .. }) {
            let pair = (self.clone(), expected.clone());
            if ASSUMED.with(|assumed| assumed.borrow().contains(&pair)) {
                return true;
            }
            ASSUMED.with(|assumed| assumed.borrow_mut().push(pair));
            let assignable = self.unfolded().is_assignable_to(&expected.unfolded());
            ASSUMED.with(|assumed| {
                assumed.borrow_mut().pop();
            });
            return assignable;
        }
        if let Self::Union(members) = self {
            return members
                .iter()
                .all(|member| member.is_assignable_to(expected));
        }
        match expected {
            Self::Unknown => true,
            Self::Any => !self.includes_nil(),
            Self::Union(members) => members.iter().any(|member| self.is_assignable_to(member)),
            Self::List(expected) => match self {
                Self::List(actual) => {
                    optional_covariant_assignable(actual.as_deref(), expected.as_deref())
                }
                // A tuple is a list of its elements.
                Self::Tuple(actual) => expected.as_deref().is_none_or(|expected| {
                    actual
                        .iter()
                        .all(|actual| actual.is_assignable_to(expected))
                }),
                _ => false,
            },
            Self::Map(expected) => match self {
                Self::Map(actual) => optional_pair_assignable(actual.as_ref(), expected.as_ref()),
                Self::Record { map: true, .. } => {
                    self.as_map().is_assignable_to(&Self::Map(expected.clone()))
                }
                _ => false,
            },
            Self::Record { fields, map } => self.conforms_to_record(fields, *map),
            Self::Function(expected) => match self {
                Self::Function(actual) => {
                    optional_signature_assignable(actual.as_deref(), expected.as_deref())
                }
                _ => false,
            },
            Self::Task(expected) => match self {
                Self::Task(actual) => {
                    optional_covariant_assignable(actual.as_deref(), expected.as_deref())
                }
                _ => false,
            },
            Self::Channel(expected) => match self {
                Self::Channel(actual) => {
                    optional_argument_assignable(actual.as_deref(), expected.as_deref())
                }
                _ => false,
            },
            Self::Resource(expected) => {
                matches!(self, Self::Resource(actual) if actual == expected)
            }
            Self::Schema => matches!(self, Self::Schema),
            Self::Struct(expected, expected_arguments) => match self {
                Self::Struct(actual, actual_arguments) => {
                    expected.is_none()
                        || (actual == expected
                            && (expected_arguments.is_empty()
                                || actual_arguments.is_empty()
                                || actual_arguments
                                    .iter()
                                    .zip(expected_arguments)
                                    .all(|(actual, expected)| actual.is_assignable_to(expected))))
                }
                _ => false,
            },
            Self::Tuple(expected) => match self {
                Self::Tuple(actual) => {
                    actual.len() == expected.len()
                        && actual
                            .iter()
                            .zip(expected)
                            .all(|(actual, expected)| actual.is_assignable_to(expected))
                }
                _ => false,
            },
            _ => false,
        }
    }
}

impl Type {
    /// Why a value of this type does not conform to `expected`, a record type,
    /// naming the first field that does not: `None` when `expected` is not a
    /// record type or this type is not a map, struct, or record.
    pub(super) fn record_mismatch(&self, expected: &Self) -> Option<String> {
        let Self::Record {
            fields: expected,
            map: false,
        } = expected.unfolded()
        else {
            return None;
        };
        let schema = match self {
            Self::Struct(Some(identity), arguments) => schema_fields(identity, arguments),
            _ => None,
        };
        expected.iter().find_map(|(name, expected)| {
            let shown = record_field_name(name);
            let actual = match self {
                Self::Record { map, .. } => match self.record_field(name) {
                    Some(actual) => actual.clone(),
                    None if *map && Self::Nil.is_assignable_to(expected) => return None,
                    None => return Some(format!("field `{shown}` is missing")),
                },
                Self::Map(Some((_, value))) => {
                    let value = Self::union([value.as_ref().clone(), Self::Nil]);
                    if value.is_assignable_to(expected) {
                        return None;
                    }
                    return Some(if value.without_nil().is_assignable_to(expected) {
                        format!(
                            "field `{shown}` may be missing, which reads `nil`, but must be {expected}"
                        )
                    } else {
                        format!("field `{shown}` would be {value}, but must be {expected}")
                    });
                }
                Self::Struct(Some(_), _) => match schema.as_ref()?.get(name) {
                    Some(actual) => actual.clone(),
                    None => return Some(format!("the struct has no field `{shown}`")),
                },
                _ => return None,
            };
            (!actual.is_assignable_to(expected))
                .then(|| format!("field `{shown}` is {actual}, but must be {expected}"))
        })
    }

    /// Whether a value of this type conforms to the record type of
    /// `expected` fields; with `exact`, to a map literal's record type.
    fn conforms_to_record(&self, expected: &[(String, Self)], exact: bool) -> bool {
        match self {
            Self::Record { fields, map } => {
                if exact {
                    return *map
                        && fields.len() == expected.len()
                        && fields.iter().zip(expected).all(
                            |((name, actual), (expected_name, expected))| {
                                name == expected_name && actual.is_assignable_to(expected)
                            },
                        );
                }
                expected.iter().all(|(name, expected)| {
                    match self.record_field(name) {
                        Some(actual) => actual.is_assignable_to(expected),
                        // A map literal without the key reads `nil` there; an
                        // open record may hold anything there.
                        None if *map => Self::Nil.is_assignable_to(expected),
                        None => Self::universal().is_assignable_to(expected),
                    }
                })
            }
            // A map or struct whose contents the checker does not know.
            Self::Map(None) => true,
            Self::Struct(None, _) => !exact,
            // A map may lack any key, which then reads `nil`.
            Self::Map(Some((_, value))) => {
                let value = Self::union([value.as_ref().clone(), Self::Nil]);
                !exact
                    && expected
                        .iter()
                        .all(|(_, expected)| value.is_assignable_to(expected))
            }
            Self::Struct(Some(identity), arguments) => {
                !exact
                    && schema_fields(identity, arguments).is_none_or(|fields| {
                        expected.iter().all(|(name, expected)| {
                            fields
                                .get(name)
                                .is_some_and(|actual| actual.is_assignable_to(expected))
                        })
                    })
            }
            _ => false,
        }
    }
}

fn optional_argument_assignable(actual: Option<&Type>, expected: Option<&Type>) -> bool {
    match (actual, expected) {
        (_, None) | (None | Some(Type::Unknown), Some(_)) => true,
        (Some(actual), Some(expected)) => actual == expected,
    }
}

/// Lists, maps, and tasks only give their elements out, so one whose elements
/// fit the expected element type fits; a channel, which also takes them in,
/// matches exactly.
fn optional_covariant_assignable(actual: Option<&Type>, expected: Option<&Type>) -> bool {
    match (actual, expected) {
        (_, None) | (None | Some(Type::Unknown), Some(_)) => true,
        (Some(actual), Some(expected)) => actual.is_assignable_to(expected),
    }
}

fn optional_pair_assignable(
    actual: Option<&(Box<Type>, Box<Type>)>,
    expected: Option<&(Box<Type>, Box<Type>)>,
) -> bool {
    match (actual, expected) {
        (_, None) | (None, Some(_)) => true,
        (Some(actual), Some(expected)) => {
            actual.0.is_assignable_to(&expected.0) && actual.1.is_assignable_to(&expected.1)
        }
    }
}

/// Signatures match position by position, where `unknown` on either side, a
/// result or parameter the checker could not infer, such as a recursive
/// function's own result, matches any type.
fn optional_signature_assignable(actual: Option<&[Type]>, expected: Option<&[Type]>) -> bool {
    match (actual, expected) {
        (_, None) | (None, Some(_)) => true,
        (Some(actual), Some(expected)) => {
            actual.len() == expected.len()
                && actual.iter().zip(expected).all(|(actual, expected)| {
                    actual == expected
                        || matches!(actual, Type::Unknown)
                        || matches!(expected, Type::Unknown)
                })
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Never => formatter.write_str("never"),
            Self::Unknown => formatter.write_str("unknown"),
            Self::Any => formatter.write_str("any"),
            Self::Nil => formatter.write_str("nil"),
            Self::Bool => formatter.write_str("bool"),
            Self::Num => formatter.write_str("num"),
            Self::Str => formatter.write_str("str"),
            Self::Bytes => formatter.write_str("bytes"),
            Self::Resource(identity) | Self::Enum(identity) => formatter.write_str(&identity.name),
            Self::List(argument) => display_application(formatter, "list", argument.as_deref()),
            Self::Map(arguments) => {
                if let Some((key, value)) = arguments {
                    write!(formatter, "map<{key}, {value}>")
                } else {
                    formatter.write_str("map")
                }
            }
            Self::Function(signature) => {
                if let Some(signature) = signature {
                    write!(
                        formatter,
                        "fn<{}>",
                        signature
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                } else {
                    formatter.write_str("fn")
                }
            }
            Self::Task(argument) => display_application(formatter, "task", argument.as_deref()),
            Self::Channel(argument) => display_application(formatter, "chan", argument.as_deref()),
            Self::Schema => formatter.write_str("schema"),
            Self::Struct(name, arguments) => match name {
                Some(name) if arguments.is_empty() => write!(formatter, "struct<{}>", name.name),
                Some(name) => write!(
                    formatter,
                    "struct<{}<{}>>",
                    name.name,
                    arguments
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                None => formatter.write_str("struct"),
            },
            Self::Tuple(elements) => write!(
                formatter,
                "[{}]",
                elements
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Record { fields, .. } => write!(
                formatter,
                "{{{}}}",
                fields
                    .iter()
                    .map(|(name, value)| format!("{}: {value}", record_field_name(name)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Generic(index) => write!(formatter, "T{index}"),
            Self::Recursive { name, .. } | Self::RecursionVar(name) => formatter.write_str(name),
            Self::AliasApplication(name, arguments) => write!(
                formatter,
                "{name}<{}>",
                arguments
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Union(members) => write!(
                formatter,
                "{}",
                members
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("|")
            ),
        }
    }
}

/// A record field's name as source writes it: bare when it is a name, quoted
/// otherwise.
fn record_field_name(name: &str) -> String {
    let mut characters = name.chars();
    let bare = characters
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && characters.all(|character| character.is_alphanumeric() || character == '_');
    if bare {
        name.to_owned()
    } else {
        format!("{name:?}")
    }
}

fn display_application(
    formatter: &mut fmt::Formatter<'_>,
    name: &str,
    argument: Option<&Type>,
) -> fmt::Result {
    if let Some(argument) = argument {
        write!(formatter, "{name}<{argument}>")
    } else {
        formatter.write_str(name)
    }
}

fn sort_key(value: &Type) -> (bool, String) {
    (matches!(value, Type::Nil), value.to_string())
}

pub(super) fn resolve_annotation(
    annotation: &TypeAnnotation,
    type_parameters: &[String],
    span: &SourceSpan,
) -> Result<Type, SourceError> {
    super::stack::grow(|| match annotation {
        TypeAnnotation::Name(name) => Ok(resolve_name(name, type_parameters)),
        TypeAnnotation::Apply { name, arguments } => {
            resolve_application(name, arguments, type_parameters, span)
        }
        TypeAnnotation::Tuple(elements) => elements
            .iter()
            .map(|element| resolve_annotation(element, type_parameters, span))
            .collect::<Result<Vec<_>, _>>()
            .map(Type::Tuple),
        TypeAnnotation::Union(members) => members
            .iter()
            .map(|member| resolve_annotation(member, type_parameters, span))
            .collect::<Result<Vec<_>, _>>()
            .map(Type::union),
        TypeAnnotation::Record(fields) => fields
            .iter()
            .map(|(name, annotation)| {
                Ok((
                    name.clone(),
                    resolve_annotation(annotation, type_parameters, span)?,
                ))
            })
            .collect::<Result<Vec<_>, SourceError>>()
            .map(|fields| Type::record(fields, false)),
    })
}

/// Resolves a source annotation for static checking, including lexical schema
/// references carried by `struct<S>`.
pub(super) fn resolve_static_annotation(
    annotation: &TypeAnnotation,
    type_parameters: &[String],
    span: &SourceSpan,
    environment: &Environment,
) -> Result<Type, SourceError> {
    reject_alias_applications(annotation, span, environment)?;
    resolve_schema_references(
        resolve_annotation(annotation, type_parameters, span)?,
        span,
        environment,
    )
}

fn reject_alias_applications(
    annotation: &TypeAnnotation,
    span: &SourceSpan,
    environment: &Environment,
) -> Result<(), SourceError> {
    super::stack::grow(|| {
        match annotation {
            TypeAnnotation::Apply { name, arguments } => {
                if matches!(
                    environment.type_member(name),
                    Some(super::environment::TypeMember::Alias(_))
                ) {
                    return Err(SourceError::semantic(
                        SourceErrorCode::InvalidTypeArguments,
                        format!("type alias `{name}` cannot accept type arguments"),
                        span.clone(),
                    ));
                }
                for argument in arguments {
                    reject_alias_applications(argument, span, environment)?;
                }
            }
            TypeAnnotation::Tuple(elements) | TypeAnnotation::Union(elements) => {
                for element in elements {
                    reject_alias_applications(element, span, environment)?;
                }
            }
            TypeAnnotation::Record(fields) => {
                for (_, annotation) in fields {
                    reject_alias_applications(annotation, span, environment)?;
                }
            }
            TypeAnnotation::Name(_) => {}
        }
        Ok(())
    })
}

/// Resolves only resource names for constraints that must carry their nominal
/// identity into private match metadata. Other constraint forms retain their
/// ordinary annotation resolution behavior.
pub(super) fn resolve_resource_references(
    value_type: Type,
    span: &SourceSpan,
    environment: &Environment,
) -> Result<Type, SourceError> {
    match value_type {
        Type::Resource(identity) if identity.is_unresolved() => {
            match environment.type_member(&identity.name) {
                Some(super::environment::TypeMember::Enum { identity, .. }) => {
                    Ok(Type::Enum(identity.clone()))
                }
                Some(super::environment::TypeMember::Alias(value_type)) => {
                    resolve_resource_references(value_type.clone(), span, environment)
                }
                Some(super::environment::TypeMember::GenericAlias { parameters, .. }) => {
                    Err(missing_alias_arguments(&identity.name, *parameters, span))
                }
                _ => environment
                    .resolve_resource_type(&identity.name, span)
                    .map(Type::Resource),
            }
        }
        Type::Recursive { name, body } => Ok(Type::Recursive {
            name,
            body: Box::new(resolve_resource_references(*body, span, environment)?),
        }),
        Type::AliasApplication(name, arguments) => {
            let arguments = arguments
                .into_iter()
                .map(|argument| resolve_resource_references(argument, span, environment))
                .collect::<Result<Vec<_>, _>>()?;
            resolve_resource_references(
                expand_alias_application(&name, &arguments, span, environment)?,
                span,
                environment,
            )
        }
        Type::Function(signature) => signature
            .map(|types| {
                types
                    .into_iter()
                    .map(|value| resolve_resource_references(value, span, environment))
                    .collect()
            })
            .transpose()
            .map(Type::Function),
        Type::Task(result) => result
            .map(|result| resolve_resource_references(*result, span, environment).map(Box::new))
            .transpose()
            .map(Type::Task),
        Type::Channel(value) => value
            .map(|value| resolve_resource_references(*value, span, environment).map(Box::new))
            .transpose()
            .map(Type::Channel),
        Type::Tuple(values) => values
            .into_iter()
            .map(|value| resolve_resource_references(value, span, environment))
            .collect::<Result<Vec<_>, _>>()
            .map(Type::Tuple),
        Type::Record { fields, map } => map_record_fields(fields, map, |value| {
            resolve_resource_references(value, span, environment)
        }),
        Type::Struct(identity, arguments) => arguments
            .into_iter()
            .map(|argument| resolve_resource_references(argument, span, environment))
            .collect::<Result<Vec<_>, _>>()
            .map(|arguments| Type::Struct(identity, arguments)),
        Type::List(element) => element
            .map(|element| resolve_resource_references(*element, span, environment).map(Box::new))
            .transpose()
            .map(Type::List),
        Type::Map(entries) => entries
            .map(|(key, value)| {
                Ok::<_, SourceError>((
                    Box::new(resolve_resource_references(*key, span, environment)?),
                    Box::new(resolve_resource_references(*value, span, environment)?),
                ))
            })
            .transpose()
            .map(Type::Map),
        Type::Union(values) => values
            .into_iter()
            .map(|value| resolve_resource_references(value, span, environment))
            .collect::<Result<Vec<_>, _>>()
            .map(Type::union),
        other => Ok(other),
    }
}

/// `struct<name<arguments>>` with `name` looked up as a schema binding.
fn resolve_struct_reference(
    name: &str,
    arguments: Vec<Type>,
    span: &SourceSpan,
    environment: &Environment,
) -> Result<Type, SourceError> {
    let arguments = arguments
        .into_iter()
        .map(|argument| resolve_schema_references(argument, span, environment))
        .collect::<Result<Vec<_>, _>>()?;
    let binding = environment.lookup(name).ok_or_else(|| {
        SourceError::semantic(
            SourceErrorCode::InvalidTypeArguments,
            format!("struct type argument `{name}` must resolve to a schema binding"),
            span.clone(),
        )
    })?;
    binding.schema_identity.clone().map_or_else(
        || {
            Err(SourceError::semantic(
                SourceErrorCode::InvalidTypeArguments,
                format!("struct type argument `{name}` must resolve to a schema binding"),
                span.clone(),
            ))
        },
        |mut resolved| {
            if !arguments.is_empty() && arguments.len() != binding.schema_arity {
                return Err(SourceError::semantic(
                    SourceErrorCode::InvalidTypeArguments,
                    if binding.schema_arity == 0 {
                        format!("schema `{name}` takes no type arguments")
                    } else {
                        format!(
                            "schema `{name}` takes {} type argument(s)",
                            binding.schema_arity
                        )
                    },
                    span.clone(),
                ));
            }
            name.clone_into(&mut resolved.name);
            Ok(Type::Struct(Some(resolved), arguments))
        },
    )
}

pub(super) fn resolve_schema_references(
    value_type: Type,
    span: &SourceSpan,
    environment: &Environment,
) -> Result<Type, SourceError> {
    match value_type {
        Type::Resource(identity) if identity.is_unresolved() => {
            match environment.type_member(&identity.name) {
                Some(super::environment::TypeMember::Enum { identity, .. }) => {
                    Ok(Type::Enum(identity.clone()))
                }
                Some(super::environment::TypeMember::Alias(value_type)) => {
                    resolve_schema_references(value_type.clone(), span, environment)
                }
                Some(super::environment::TypeMember::GenericAlias { parameters, .. }) => {
                    Err(missing_alias_arguments(&identity.name, *parameters, span))
                }
                _ => environment
                    .resolve_resource_type(&identity.name, span)
                    .map(Type::Resource),
            }
        }
        Type::Recursive { name, body } => Ok(Type::Recursive {
            name,
            body: Box::new(resolve_schema_references(*body, span, environment)?),
        }),
        Type::AliasApplication(name, arguments) => {
            let arguments = arguments
                .into_iter()
                .map(|argument| resolve_schema_references(argument, span, environment))
                .collect::<Result<Vec<_>, _>>()?;
            resolve_schema_references(
                expand_alias_application(&name, &arguments, span, environment)?,
                span,
                environment,
            )
        }
        Type::Struct(Some(identity), arguments) if identity.is_unresolved() => {
            resolve_struct_reference(&identity.name, arguments, span, environment)
        }
        Type::List(element) => element
            .map(|element| resolve_schema_references(*element, span, environment).map(Box::new))
            .transpose()
            .map(Type::List),
        Type::Map(entries) => entries
            .map(|(key, value)| {
                Ok::<_, SourceError>((
                    Box::new(resolve_schema_references(*key, span, environment)?),
                    Box::new(resolve_schema_references(*value, span, environment)?),
                ))
            })
            .transpose()
            .map(Type::Map),
        Type::Function(signature) => signature
            .map(|types| {
                types
                    .into_iter()
                    .map(|value| resolve_schema_references(value, span, environment))
                    .collect()
            })
            .transpose()
            .map(Type::Function),
        Type::Task(result) => result
            .map(|result| resolve_schema_references(*result, span, environment).map(Box::new))
            .transpose()
            .map(Type::Task),
        Type::Channel(value) => value
            .map(|value| resolve_schema_references(*value, span, environment).map(Box::new))
            .transpose()
            .map(Type::Channel),
        Type::Tuple(values) => values
            .into_iter()
            .map(|value| resolve_schema_references(value, span, environment))
            .collect::<Result<Vec<_>, _>>()
            .map(Type::Tuple),
        Type::Record { fields, map } => map_record_fields(fields, map, |value| {
            resolve_schema_references(value, span, environment)
        }),
        Type::Union(values) => values
            .into_iter()
            .map(|value| resolve_schema_references(value, span, environment))
            .collect::<Result<Vec<_>, _>>()
            .map(Type::union),
        other => Ok(other),
    }
}

/// A record type of `fields`, each with its type resolved by `resolve`.
fn map_record_fields(
    fields: Vec<(String, Type)>,
    map: bool,
    resolve: impl Fn(Type) -> Result<Type, SourceError>,
) -> Result<Type, SourceError> {
    fields
        .into_iter()
        .map(|(name, value)| Ok((name, resolve(value)?)))
        .collect::<Result<Vec<_>, _>>()
        .map(|fields| Type::Record { fields, map })
}

fn resolve_name(name: &str, type_parameters: &[String]) -> Type {
    if let Some(index) = type_parameters
        .iter()
        .position(|parameter| parameter == name)
    {
        return Type::Generic(index);
    }
    match name {
        "any" => Type::Any,
        "nil" => Type::Nil,
        "bool" => Type::Bool,
        "num" => Type::Num,
        "str" => Type::Str,
        "bytes" => Type::Bytes,
        "list" => Type::List(None),
        "map" => Type::Map(None),
        "fn" => Type::Function(None),
        "task" => Type::Task(None),
        "chan" => Type::Channel(None),
        "schema" => Type::Schema,
        "struct" => Type::Struct(None, Vec::new()),
        _ => Type::Resource(ResourceIdentity::unresolved(name)),
    }
}

fn resolve_application(
    name: &str,
    arguments: &[TypeAnnotation],
    type_parameters: &[String],
    span: &SourceSpan,
) -> Result<Type, SourceError> {
    super::stack::grow(|| {
        if name == "struct" {
            if arguments.len() != 1 {
                return Err(wrong_type_arity(name, "1", span));
            }
            // `struct<S>`, or `struct<S<T, ...>>` for a generic schema.
            let (name, schema_arguments) = match &arguments[0] {
                TypeAnnotation::Name(name) => (name, Vec::new()),
                TypeAnnotation::Apply { name, arguments } => (
                    name,
                    arguments
                        .iter()
                        .map(|argument| resolve_annotation(argument, type_parameters, span))
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                _ => {
                    return Err(SourceError::semantic(
                        SourceErrorCode::InvalidTypeArguments,
                        "struct type argument must be a schema name",
                        span.clone(),
                    ));
                }
            };
            return Ok(Type::Struct(
                Some(SchemaIdentity::unresolved(name)),
                schema_arguments,
            ));
        }
        let resolved = arguments
            .iter()
            .map(|argument| resolve_annotation(argument, type_parameters, span))
            .collect::<Result<Vec<_>, _>>()?;
        match (name, resolved.as_slice()) {
            ("list", [element]) => Ok(Type::List(Some(Box::new(element.clone())))),
            ("map", [key, value]) => Ok(Type::Map(Some((
                Box::new(key.clone()),
                Box::new(value.clone()),
            )))),
            ("fn", [_, ..]) => Ok(Type::Function(Some(resolved))),
            ("task", [result]) => Ok(Type::Task(Some(Box::new(result.clone())))),
            ("chan", [value]) => Ok(Type::Channel(Some(Box::new(value.clone())))),
            ("list" | "task" | "chan", _) => Err(wrong_type_arity(name, "1", span)),
            ("map", _) => Err(wrong_type_arity(name, "2", span)),
            ("fn", _) => Err(wrong_type_arity(name, "at least 1", span)),
            ("schema", _) => Err(wrong_type_arity(name, "0", span)),
            // A generic alias, looked up when the annotation is resolved.
            _ => Ok(Type::AliasApplication(name.into(), resolved)),
        }
    })
}

/// The type a use of generic alias `name` with `arguments` stands for: its
/// body with each parameter filled in.
fn expand_alias_application(
    name: &str,
    arguments: &[Type],
    span: &SourceSpan,
    environment: &Environment,
) -> Result<Type, SourceError> {
    match environment.type_member(name) {
        Some(super::environment::TypeMember::GenericAlias { parameters, body }) => {
            if arguments.len() != *parameters {
                return Err(SourceError::semantic(
                    SourceErrorCode::InvalidTypeArguments,
                    format!(
                        "type alias `{name}` takes {parameters} type argument(s), got {}",
                        arguments.len()
                    ),
                    span.clone(),
                ));
            }
            let substitutions = arguments.iter().cloned().enumerate().collect();
            Ok(substitute(body, &substitutions))
        }
        Some(super::environment::TypeMember::Alias(_)) => Err(SourceError::semantic(
            SourceErrorCode::InvalidTypeArguments,
            format!("type alias `{name}` cannot accept type arguments"),
            span.clone(),
        )),
        _ => Err(SourceError::semantic(
            SourceErrorCode::UnknownType,
            format!("unknown type constructor `{name}`"),
            span.clone(),
        )),
    }
}

/// The error for a generic alias named without its type arguments.
fn missing_alias_arguments(name: &str, parameters: usize, span: &SourceSpan) -> SourceError {
    SourceError::semantic(
        SourceErrorCode::InvalidTypeArguments,
        format!("type alias `{name}` needs {parameters} type argument(s)"),
        span.clone(),
    )
}

/// `value_type` with each `Type::Generic(i)` replaced by substitution `i`, or
/// by `unknown` when there is none.
pub(super) fn substitute(value_type: &Type, substitutions: &HashMap<usize, Type>) -> Type {
    match value_type {
        Type::Generic(index) => substitutions.get(index).cloned().unwrap_or(Type::Unknown),
        Type::List(argument) => Type::List(
            argument
                .as_ref()
                .map(|argument| Box::new(substitute(argument, substitutions))),
        ),
        Type::Map(arguments) => Type::Map(arguments.as_ref().map(|(key, value)| {
            (
                Box::new(substitute(key, substitutions)),
                Box::new(substitute(value, substitutions)),
            )
        })),
        Type::Function(signature) => Type::Function(signature.as_ref().map(|signature| {
            signature
                .iter()
                .map(|value| substitute(value, substitutions))
                .collect()
        })),
        Type::Task(result) => Type::Task(
            result
                .as_ref()
                .map(|result| Box::new(substitute(result, substitutions))),
        ),
        Type::Channel(value) => Type::Channel(
            value
                .as_ref()
                .map(|value| Box::new(substitute(value, substitutions))),
        ),
        Type::Tuple(elements) => Type::Tuple(
            elements
                .iter()
                .map(|element| substitute(element, substitutions))
                .collect(),
        ),
        Type::Union(members) => Type::union(
            members
                .iter()
                .map(|member| substitute(member, substitutions)),
        ),
        Type::AliasApplication(name, arguments) => Type::AliasApplication(
            name.clone(),
            arguments
                .iter()
                .map(|argument| substitute(argument, substitutions))
                .collect(),
        ),
        Type::Recursive { name, body } => Type::Recursive {
            name: name.clone(),
            body: Box::new(substitute(body, substitutions)),
        },
        Type::Record { fields, map } => Type::Record {
            fields: fields
                .iter()
                .map(|(name, value)| (name.clone(), substitute(value, substitutions)))
                .collect(),
            map: *map,
        },
        Type::Struct(identity, arguments) => Type::Struct(
            identity.clone(),
            arguments
                .iter()
                .map(|argument| substitute(argument, substitutions))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn wrong_type_arity(name: &str, expected: &str, span: &SourceSpan) -> SourceError {
    SourceError::semantic(
        SourceErrorCode::InvalidTypeArguments,
        format!("type `{name}` expects {expected} argument(s)"),
        span.clone(),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{AlternativeType, AlternativeVariable, InferredAlternative, NominalIdentity, Type};

    #[test]
    fn inferred_alternatives_canonicalize_scoped_symbols() {
        let first = InferredAlternative::new(
            vec![
                AlternativeType::list(AlternativeType::variable(AlternativeVariable::new(7))),
                AlternativeType::list(AlternativeType::variable(AlternativeVariable::new(3))),
            ],
            AlternativeType::list(AlternativeType::union([
                AlternativeType::variable(AlternativeVariable::new(3)),
                AlternativeType::variable(AlternativeVariable::new(7)),
            ])),
        );
        let equivalent = InferredAlternative::new(
            vec![
                AlternativeType::list(AlternativeType::variable(AlternativeVariable::new(42))),
                AlternativeType::list(AlternativeType::variable(AlternativeVariable::new(99))),
            ],
            AlternativeType::list(AlternativeType::union([
                AlternativeType::variable(AlternativeVariable::new(99)),
                AlternativeType::variable(AlternativeVariable::new(42)),
            ])),
        );

        assert_eq!(first, equivalent);
        assert_eq!(first.to_string(), "(list<A0>, list<A1>) -> list<A0|A1>");
    }

    #[test]
    fn inferred_alternatives_substitute_symbols_without_source_generics() {
        let alternative = InferredAlternative::new(
            vec![
                AlternativeType::map(
                    AlternativeType::variable(AlternativeVariable::new(4)),
                    AlternativeType::variable(AlternativeVariable::new(5)),
                ),
                AlternativeType::concrete(Type::Generic(0)),
            ],
            AlternativeType::variable(AlternativeVariable::new(5)),
        );

        let (parameters, result) = alternative.substitute(&HashMap::from([
            (AlternativeVariable::new(0), Type::Str),
            (AlternativeVariable::new(1), Type::Num),
        ]));

        assert_eq!(
            parameters,
            vec![
                Type::Map(Some((Box::new(Type::Str), Box::new(Type::Num)))),
                Type::Generic(0),
            ]
        );
        assert_eq!(result, Type::Num);
    }

    #[test]
    fn any_excludes_nil_and_any_nil_is_universal() {
        let universal = Type::universal();

        assert!(Type::Str.is_assignable_to(&Type::Any));
        assert!(!Type::Nil.is_assignable_to(&Type::Any));
        assert!(Type::Str.is_assignable_to(&universal));
        assert!(Type::Nil.is_assignable_to(&universal));
    }

    #[test]
    fn unions_are_canonical_and_absorb_redundant_members() {
        assert_eq!(Type::union([Type::Any, Type::Str]), Type::Any);
        assert_eq!(
            Type::union([Type::Str, Type::Nil, Type::Any]),
            Type::universal()
        );
        assert_eq!(
            Type::union([Type::Str, Type::Nil, Type::Str]),
            Type::Union(vec![Type::Str, Type::Nil])
        );
        assert_eq!(Type::union([Type::Unknown, Type::Any]), Type::Unknown);
    }

    #[test]
    fn unions_normalize_nested_nominal_and_non_returning_members() {
        assert_eq!(Type::union([Type::Str, Type::Str]), Type::Str);
        assert_eq!(
            Type::union([Type::Str, Type::union([Type::Num, Type::Str])]),
            Type::Union(vec![Type::Num, Type::Str])
        );

        let file = Type::Resource(NominalIdentity::declared("slug.io.fs", "File"));
        assert_eq!(Type::union([file.clone(), file.clone()]), file);

        assert_eq!(Type::union([Type::Never, Type::Str]), Type::Str);
        assert_eq!(Type::union(Vec::new()), Type::Never);
        assert_eq!(Type::union([Type::Never, Type::Unknown]), Type::Unknown);
        assert_eq!(
            Type::union([Type::Never, Type::Any, Type::Nil]),
            Type::universal()
        );

        let repeated = (0..8).fold(Type::Never, |value, _| Type::union([value, Type::Str]));
        assert_eq!(repeated, Type::Str);
    }

    #[test]
    fn structured_types_are_reflexive() {
        let list = Type::List(Some(Box::new(Type::Str)));
        let tuple = Type::Tuple(vec![Type::Str, Type::Num]);

        assert!(list.is_assignable_to(&list));
        assert!(tuple.is_assignable_to(&tuple));
        assert!(Type::universal().is_assignable_to(&Type::universal()));
    }

    #[test]
    fn compound_types_retain_nominal_resource_identity() {
        let file = Type::Resource(NominalIdentity::declared("test", "File"));
        let socket = Type::Resource(NominalIdentity::declared("test", "Socket"));

        for wrap in [
            Type::List as fn(Option<Box<Type>>) -> Type,
            Type::Channel,
            Type::Task,
        ] {
            let files = wrap(Some(Box::new(file.clone())));
            let sockets = wrap(Some(Box::new(socket.clone())));
            assert!(files.is_assignable_to(&files));
            assert!(!files.is_assignable_to(&sockets));
            assert!(!sockets.is_assignable_to(&files));
        }

        let maybe_file = Type::union([file.clone(), Type::Nil]);
        let maybe_socket = Type::union([socket.clone(), Type::Nil]);
        assert!(!maybe_file.is_assignable_to(&maybe_socket));
        assert!(
            !Type::List(Some(Box::new(maybe_file)))
                .is_assignable_to(&Type::List(Some(Box::new(maybe_socket))))
        );
    }

    #[test]
    fn invariant_compounds_do_not_bypass_nominal_assignability() {
        let file = Type::Resource(NominalIdentity::declared("test", "File"));
        let socket = Type::Resource(NominalIdentity::declared("test", "Socket"));
        let file_map = Type::Map(Some((Box::new(Type::Str), Box::new(file.clone()))));
        let socket_map = Type::Map(Some((Box::new(Type::Str), Box::new(socket.clone()))));
        let file_function = Type::Function(Some(vec![file.clone(), file]));
        let socket_function = Type::Function(Some(vec![socket.clone(), socket]));

        assert!(!file_map.is_assignable_to(&socket_map));
        assert!(!socket_map.is_assignable_to(&file_map));
        assert!(!file_function.is_assignable_to(&socket_function));
        assert!(!socket_function.is_assignable_to(&file_function));
    }
}
