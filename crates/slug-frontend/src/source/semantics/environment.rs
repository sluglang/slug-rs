use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use slug_vm::{CallableIdentity, ForeignResourceSignature, SourceSpan};

use super::semantic::{EnumIdentity, InferredAlternative, ResourceIdentity, SchemaIdentity, Type};

#[derive(Clone)]
pub(super) struct CallableParameter {
    pub(super) label: Option<String>,
    pub(super) value_type: Type,
    pub(super) has_default: bool,
    pub(super) variadic: bool,
    /// Whether `value_type` is what the function's body relies on, without
    /// an annotation, which a failed check of it says. It is not part of the
    /// signature: two that differ only in it are the same, with the same
    /// identity.
    pub(super) inferred: bool,
}

impl PartialEq for CallableParameter {
    fn eq(&self, other: &Self) -> bool {
        self.label == other.label
            && self.value_type == other.value_type
            && self.has_default == other.has_default
            && self.variadic == other.variadic
    }
}

impl Eq for CallableParameter {}

// A signature's identity is built from this, so it leaves `inferred` out.
impl std::fmt::Debug for CallableParameter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CallableParameter")
            .field("label", &self.label)
            .field("value_type", &self.value_type)
            .field("has_default", &self.has_default)
            .field("variadic", &self.variadic)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub(super) struct CallableSignature {
    pub(super) generic_arity: usize,
    /// A generic signature's type parameters' names, for diagnostics.
    pub(super) type_parameter_names: Vec<String>,
    pub(super) parameters: Vec<CallableParameter>,
    pub(super) result: Type,
    pub(super) inferred_alternatives: Vec<InferredAlternative>,
    /// Whether a value the checker could not prove may reach a type
    /// parameter in the result, where the function cannot check it: a
    /// caller then checks the result it instantiates. `None` until the
    /// function's body is checked, for a call that precedes it.
    pub(super) result_unproven: Option<bool>,
    /// Whether a generic function only ever passes its function-typed
    /// parameters values of the types its own parameters give, and never lets
    /// them escape: then a function literal given to it, whose other
    /// arguments are proven, need not check its own parameters.
    pub(super) parametric: bool,
}

impl CallableSignature {
    pub(super) fn has_same_input(&self, other: &Self) -> bool {
        self.generic_arity == other.generic_arity
            && self.parameters == other.parameters
            && self.inferred_alternatives == other.inferred_alternatives
    }

    pub(super) fn identity(&self) -> CallableIdentity {
        CallableIdentity::new(format!(
            "{:?}:{:?}:{:?}",
            self.generic_arity, self.parameters, self.inferred_alternatives
        ))
    }
}

pub(super) fn foreign_resource_signature(
    signature: &CallableSignature,
) -> ForeignResourceSignature {
    let resource = |identity: &ResourceIdentity| {
        (
            identity.explicit_runtime_module().map(str::to_owned),
            identity.name.clone(),
        )
    };
    ForeignResourceSignature::new(
        signature
            .parameters
            .iter()
            .map(|parameter| match &parameter.value_type {
                Type::Resource(identity) => Some(resource(identity)),
                _ => None,
            })
            .collect(),
        match &signature.result {
            Type::Resource(identity) => Some(resource(identity)),
            _ => None,
        },
    )
}

/// A map named by an immutable binding and constant fields, as
/// `empire.kills`: it is the same map wherever the binding is the same one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct KeyPath {
    pub(super) root: String,
    /// The root's binding, by address: a name bound again is another map.
    pub(super) binding: usize,
    pub(super) fields: Vec<String>,
}

/// A binding's function, as far as a call entering past its checks cares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum FunctionKind {
    /// Anything else: a value, or a function as its declaration gives it.
    #[default]
    Ordinary,
    /// A function declared later in its block or module, by the signature
    /// its annotations give, every parameter annotated: a call before the
    /// declaration calls what that signature describes.
    Predeclared,
    /// A `foreign` function, which the host calls and whose arguments it
    /// checks itself, answering with its own errors.
    Foreign,
}

#[derive(Clone, Debug)]
pub(super) struct SemanticBinding {
    pub(super) value_type: Type,
    pub(super) callables: Vec<CallableSignature>,
    pub(super) members: HashMap<String, SemanticBinding>,
    pub(super) required_fields: HashSet<String>,
    pub(super) schema_identity: Option<SchemaIdentity>,
    pub(super) resource_identity: Option<ResourceIdentity>,
    pub(super) type_members: HashMap<String, TypeMember>,
    pub(super) module_binding: bool,
    /// For a module binding, the schemas its exported types name, including
    /// private ones, so an importer can check their fields.
    pub(super) schemas: Vec<SemanticBinding>,
    /// For a `var`, the type every assignment must conform to, fixed at its
    /// declaration. `value_type` is then its current type, which assignments
    /// and nil tests refine for the code that follows.
    pub(super) declared: Option<Type>,
    /// For a `var` whose first value is `nil`, where it was declared and the
    /// assignments its declared type was inferred from.
    pub(super) nil_start: Option<NilStart>,
    /// An implicit builtin, such as `close`, which a declaration of the same
    /// name replaces rather than overloads, as it does at runtime.
    pub(super) implicit: bool,
    /// What kind of function, if any, the binding is, as a proven call
    /// needs to know.
    pub(super) function_kind: FunctionKind,
    /// For a closure's parameter, the map whose keys it ranges over, as
    /// `k` in `filter(keys(m), fn(k) { ... })`: `m[k]` reads a value.
    pub(super) key_of: Option<Rc<KeyPath>>,
    /// For a `val` bound to `keys(m)`, the map `m`: an element of it is one
    /// of `m`'s keys.
    pub(super) keys_of: Option<Rc<KeyPath>>,
    /// For an import whose module has no types here, as one that does not
    /// compile, the module's name: a type named through it says so.
    pub(super) unavailable_module: Option<String>,
    /// For an immutable binding, the narrowed types of string-keyed members a
    /// nil test established on this path, such as `r.error` after
    /// `r.error != nil`. The binding never changes, so neither do they.
    pub(super) member_facts: HashMap<String, Type>,
    /// For a generic schema, how many type parameters it takes. Its fields'
    /// types refer to them as `Type::Generic(i)`.
    pub(super) schema_arity: usize,
}

#[derive(Clone, Debug)]
pub(super) enum TypeMember {
    Resource(ResourceIdentity),
    Enum {
        identity: EnumIdentity,
        cases: Vec<String>,
        /// How many type parameters a generic enum takes.
        arity: usize,
        /// The cases that carry a payload, each a schema under the enum.
        payloads: Vec<String>,
    },
    Alias(Type),
    /// A generic alias: its body, in which `Type::Generic(i)` is its `i`th
    /// parameter, filled in at each use.
    GenericAlias {
        parameters: usize,
        body: Type,
    },
}

impl TypeMember {
    pub(super) fn with_resource_runtime_module(&self, module: &str) -> Self {
        match self {
            Self::Resource(identity) => {
                let mut identity = identity.clone();
                identity.set_runtime_module(module.into());
                Self::Resource(identity)
            }
            Self::Enum {
                identity,
                cases,
                arity,
                payloads,
            } => {
                let mut identity = identity.clone();
                identity.set_runtime_module(module.into());
                Self::Enum {
                    identity,
                    cases: cases.clone(),
                    arity: *arity,
                    payloads: payloads.clone(),
                }
            }
            Self::Alias(_) | Self::GenericAlias { .. } => self.clone(),
        }
    }
}

impl SemanticBinding {
    pub(super) fn value(value_type: Type) -> Self {
        Self {
            value_type,
            callables: Vec::new(),
            members: HashMap::new(),
            required_fields: HashSet::new(),
            schema_identity: None,
            resource_identity: None,
            type_members: HashMap::new(),
            module_binding: false,
            schemas: Vec::new(),
            declared: None,
            nil_start: None,
            implicit: false,
            function_kind: FunctionKind::Ordinary,
            key_of: None,
            keys_of: None,
            unavailable_module: None,
            member_facts: HashMap::new(),
            schema_arity: 0,
        }
    }

    pub(super) fn callable(signature: CallableSignature) -> Self {
        Self {
            value_type: function_value_type(&signature),
            callables: vec![signature],
            members: HashMap::new(),
            required_fields: HashSet::new(),
            schema_identity: None,
            resource_identity: None,
            type_members: HashMap::new(),
            module_binding: false,
            schemas: Vec::new(),
            declared: None,
            nil_start: None,
            implicit: false,
            function_kind: FunctionKind::Ordinary,
            key_of: None,
            keys_of: None,
            unavailable_module: None,
            member_facts: HashMap::new(),
            schema_arity: 0,
        }
    }

    pub(super) fn module(
        members: HashMap<String, SemanticBinding>,
        type_members: HashMap<String, TypeMember>,
        schemas: Vec<SemanticBinding>,
    ) -> Self {
        Self {
            value_type: Type::Map(None),
            callables: Vec::new(),
            members,
            required_fields: HashSet::new(),
            schema_identity: None,
            resource_identity: None,
            type_members,
            module_binding: true,
            schemas,
            declared: None,
            nil_start: None,
            implicit: false,
            function_kind: FunctionKind::Ordinary,
            key_of: None,
            keys_of: None,
            unavailable_module: None,
            member_facts: HashMap::new(),
            schema_arity: 0,
        }
    }

    pub(super) fn set_resource_runtime_module(&mut self, module: &str) {
        if let Some(identity) = &mut self.resource_identity {
            identity.set_runtime_module(module.into());
        }
        for member in self.members.values_mut() {
            member.set_resource_runtime_module(module);
        }
    }
}

pub(super) fn function_value_type(signature: &CallableSignature) -> Type {
    Type::Function(Some(
        std::iter::once(signature.result.clone())
            .chain(
                signature
                    .parameters
                    .iter()
                    .map(|parameter| parameter.value_type.clone()),
            )
            .collect(),
    ))
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ModuleSnapshot {
    pub(super) exports: HashMap<String, SemanticBinding>,
    pub(super) types: HashMap<String, TypeMember>,
    /// The schemas that exported values and types name.
    pub(super) schemas: Vec<SemanticBinding>,
}

impl ModuleSnapshot {
    /// The names of the module's exported values.
    pub(crate) fn export_names(&self) -> impl Iterator<Item = &str> {
        self.exports.keys().map(String::as_str)
    }
}

/// Semantic bindings retained by one interactive source session.
#[derive(Clone, Debug, Default)]
pub(crate) struct SessionSnapshot {
    bindings: HashMap<String, SemanticBinding>,
    types: HashMap<String, TypeMember>,
}

pub(super) type ImportSnapshots = HashMap<String, ModuleSnapshot>;

/// The annotated place a runtime check guards, which its error names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CheckPlace {
    /// The value of an annotated declaration, binding `name` unless it
    /// destructures.
    Binding(Option<String>),
    /// An assignment to the annotated `var` `name`.
    Assignment(String),
    /// What a function completes with, by its body's end, `return`, or a
    /// recovering `defer onerror`.
    Result,
    /// The parameter `name`, at `index`, on entry. `inferred` when its type
    /// is not annotated but what the checker inferred inside the body, from
    /// how the body uses it or what a callee expects of the function.
    Parameter {
        name: String,
        index: usize,
        inferred: bool,
    },
    /// The field `name` of a struct being constructed or copied.
    Field(String),
    /// What a call returns where nothing enforced its type: a call through
    /// a function value, whose type describes it, or a generic call whose
    /// instantiated result the callee could not prove.
    CallResult,
    /// What a `foreign` function returns, which host code gives.
    ForeignResult,
}

/// A runtime check where a value whose type the checker could not prove
/// reaches an annotated place: the value must conform to `expected`, whose
/// type parameters a check does not test.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BoundaryCheck {
    pub(super) expected: Type,
    pub(super) place: CheckPlace,
    /// `expected` as the runtime tests it: an enum with payload cases is
    /// its fieldless cases or the structs of its payload cases.
    pub(super) runtime: Type,
}

#[derive(Clone, Debug, Default)]
pub(super) struct SemanticAnalysis {
    pub(super) snapshot: ModuleSnapshot,
    pub(super) session_snapshot: SessionSnapshot,
    pub(super) selected_calls: HashMap<SourceSpan, CallableIdentity>,
    /// The calls of a function a `val` names that give every argument
    /// positionally: such a call checks the arguments the checker cannot
    /// prove where it is, and enters past the function's entry checks.
    pub(super) proven_calls: HashSet<SourceSpan>,
    /// The function literals given to a parametric function with proven
    /// arguments and a proven result: they do not check their parameters,
    /// whose values the function passes them only from those.
    pub(super) trusted_closures: HashSet<SourceSpan>,
    /// The named functions given to a parametric function as function
    /// literals would be trusted, by how many parameters it passes them:
    /// each is called through a literal that calls it by name, past its
    /// checks.
    pub(super) trusted_references: HashMap<SourceSpan, usize>,
    pub(super) function_identities: HashMap<SourceSpan, CallableIdentity>,
    pub(super) foreign_identities: HashMap<SourceSpan, CallableIdentity>,
    pub(crate) foreign_resource_signatures: HashMap<SourceSpan, ForeignResourceSignature>,
    pub(super) match_constraints: HashMap<SourceSpan, Vec<Option<Type>>>,
    pub(super) expression_types: HashMap<SourceSpan, Type>,
    /// The runtime checks each site makes: a declaration, an assignment, a
    /// `return`, a `defer`, a function literal for its parameters and its
    /// body's result, a struct construction or copy for its fields, or a
    /// call through a function value for its result.
    pub(super) boundary_checks: HashMap<SourceSpan, Vec<BoundaryCheck>>,
    /// The names `{*}` declarations define, or `None` when one selects from
    /// a map whose entries are not known until it runs.
    pub(super) selected_globals: Option<HashSet<String>>,
}

#[derive(Clone, Debug, Default)]
struct SemanticRecords {
    /// For a call whose result an annotation describes, that type.
    expected_results: HashMap<SourceSpan, Type>,
    /// For a function body that is a `match` without a subject, the type of
    /// the arguments it matches.
    implicit_subjects: HashMap<SourceSpan, Type>,
    /// For a function literal that is an argument, the parameter types its
    /// callee expects of it.
    expected_parameters: HashMap<SourceSpan, Vec<Type>>,
    /// The result a function literal argument is expected to give, when the
    /// call's types determine it, which its last value is refined to, as a
    /// list literal to the tuple it makes.
    expected_literal_results: HashMap<SourceSpan, Type>,
    selected_calls: HashMap<SourceSpan, CallableIdentity>,
    /// Whether each call that may skip its callee's entry checks was found
    /// proven every time it was checked: a trial, or a body checked again,
    /// that finds it unproven makes it unproven for good.
    proven_calls: HashMap<SourceSpan, bool>,
    /// Whether each generic function literal was found parametric every time
    /// it was checked.
    parametric: HashMap<SourceSpan, bool>,
    /// For a function literal given with `keys(m)`, which of its parameters
    /// range over `m`'s keys, by position.
    key_parameters: HashMap<SourceSpan, Vec<(usize, KeyPath)>>,
    /// Whether each function literal given to a parametric function was
    /// found to need no checks of its parameters every time it was checked.
    trusted_closures: HashMap<SourceSpan, bool>,
    /// Whether each named function given to a parametric function was found
    /// trusted, with the number of parameters passed it, every time.
    trusted_references: HashMap<SourceSpan, Option<usize>>,
    function_identities: HashMap<SourceSpan, CallableIdentity>,
    foreign_identities: HashMap<SourceSpan, CallableIdentity>,
    foreign_resource_signatures: HashMap<SourceSpan, ForeignResourceSignature>,
    match_constraints: HashMap<SourceSpan, Vec<Option<Type>>>,
    expression_types: HashMap<SourceSpan, Type>,
    boundary_checks: HashMap<SourceSpan, Vec<BoundaryCheck>>,
    /// The function literals whose result a value the checker could not
    /// prove may reach at a type parameter.
    unproven_results: HashSet<SourceSpan>,
    /// Generic calls whose result is checked if their callee, by identity,
    /// turns out to be one of `unproven_results`, which a callee declared
    /// later in the module is not yet known to be: each as the call, its
    /// instantiated result, and the callee.
    conditional_results: Vec<(SourceSpan, Type, CallableIdentity)>,
    selected_globals: HashSet<String>,
    unknown_selection: bool,
}

/// A `var` whose first value is `nil`. Its declared type is `nil` with the
/// type of its first assignment written in its scope, which a trial check of
/// that scope finds; with none found, or one of unknown type, it is
/// `any|nil`. `assignments` holds that first assignment, if found.
#[derive(Clone, Debug)]
pub(super) struct NilStart {
    pub(super) site: NilStartSite,
    pub(super) assignments: Rc<Vec<(SourceSpan, Type)>>,
}

/// A `nil`-started `var`, by the span of its declaration and its name.
pub(super) type NilStartSite = (SourceSpan, String);

#[derive(Debug, Default)]
struct NilStarts {
    /// Present during a trial check: the assignments seen to each `var`.
    trial: Option<HashMap<NilStartSite, Vec<(SourceSpan, Type)>>>,
    /// The assignments trial checks found, for the real check to declare.
    inferred: HashMap<NilStartSite, Rc<Vec<(SourceSpan, Type)>>>,
}

#[derive(Clone, Debug)]
pub(super) struct Environment {
    /// Innermost last. A clone shares every scope and binding, and a scope
    /// or binding is copied when it is first changed (`Rc::make_mut`), so
    /// cloning for a block, branch, or function body costs one pointer per
    /// scope rather than a copy of every binding's type.
    scopes: Vec<Rc<Scope>>,
    type_scopes: Vec<Rc<HashMap<String, TypeMember>>>,
    imports: Rc<ImportSnapshots>,
    records: Rc<RefCell<SemanticRecords>>,
    /// The function body being checked, shared by every environment cloned
    /// within it, such as a match case's, and replaced inside a nested
    /// function. `None` outside any function.
    function: Option<Rc<RefCell<FunctionReturns>>>,
    nil_starts: Rc<RefCell<NilStarts>>,
}

/// One lexical scope's bindings, each shared until changed.
type Scope = HashMap<String, Rc<SemanticBinding>>;

/// What a function body's `return` expressions must and do produce.
#[derive(Clone, Debug)]
struct FunctionReturns {
    expected: Option<Type>,
    returned: Vec<Type>,
    /// Each parameter's entry check, by position: its name, the type it is
    /// checked against, and whether that type was inferred. A `recur`
    /// checks its arguments against them, so it need not re-enter them.
    parameters: Vec<Option<(String, Type, bool)>>,
    /// Whether a value the checker could not prove may reach a type
    /// parameter in the result.
    result_unproven: bool,
    /// A generic function's function-typed parameters, by name, with the
    /// types of their parameters, which each call through one must pass
    /// exactly for the function to be parametric.
    through: Vec<(String, Vec<Type>)>,
    /// Whether a call through one of `through` passed something else.
    unparametric: bool,
}

impl Environment {
    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self::with_imports_and_session(HashMap::new(), &SessionSnapshot::default())
    }

    pub(super) fn with_imports_and_session(
        imports: ImportSnapshots,
        session: &SessionSnapshot,
    ) -> Self {
        Self {
            scopes: vec![Rc::new(
                session
                    .bindings
                    .iter()
                    .map(|(name, binding)| (name.clone(), Rc::new(binding.clone())))
                    .collect(),
            )],
            type_scopes: vec![Rc::new(session.types.clone())],
            imports: Rc::new(imports),
            records: Rc::new(RefCell::new(SemanticRecords::default())),
            function: None,
            nil_starts: Rc::new(RefCell::new(NilStarts::default())),
        }
    }

    /// A copy for a trial check, sharing nothing it could record into with
    /// this environment, in which `nil`-started vars are dynamic, except
    /// those `seed` gives assignments for, and their assignments are logged.
    pub(super) fn trial(&self, seed: HashMap<NilStartSite, Rc<Vec<(SourceSpan, Type)>>>) -> Self {
        let mut trial = self.clone();
        trial.records = Rc::new(RefCell::new(self.records.borrow().clone()));
        trial.function = self
            .function
            .as_ref()
            .map(|function| Rc::new(RefCell::new(function.borrow().clone())));
        trial.nil_starts = Rc::new(RefCell::new(NilStarts {
            trial: Some(HashMap::new()),
            inferred: seed,
        }));
        trial
    }

    pub(super) fn in_trial(&self) -> bool {
        self.nil_starts.borrow().trial.is_some()
    }

    /// The assignments a finished trial logged.
    pub(super) fn trial_assignments(&self) -> HashMap<NilStartSite, Vec<(SourceSpan, Type)>> {
        self.nil_starts.borrow().trial.clone().unwrap_or_default()
    }

    /// Records the assignments a `var` declared at `site` was found to have.
    pub(super) fn infer_nil_start(&self, site: NilStartSite, assignments: Vec<(SourceSpan, Type)>) {
        self.nil_starts
            .borrow_mut()
            .inferred
            .insert(site, Rc::new(assignments));
    }

    /// Logs, during a trial, an assignment of `value_type` to `name`.
    pub(super) fn log_assignment(&self, name: &str, span: &SourceSpan, value_type: &Type) {
        let Some(site) = self
            .lookup(name)
            .and_then(|binding| binding.nil_start.as_ref())
            .map(|start| start.site.clone())
        else {
            return;
        };
        if let Some(trial) = &mut self.nil_starts.borrow_mut().trial {
            trial
                .entry(site)
                .or_default()
                .push((span.clone(), value_type.clone()));
        }
    }

    /// Begins checking a function body whose result must conform to
    /// `expected`, when it is declared.
    pub(super) fn enter_function(
        &mut self,
        expected: Option<Type>,
        parameters: Vec<Option<(String, Type, bool)>>,
    ) {
        self.function = Some(Rc::new(RefCell::new(FunctionReturns {
            expected,
            returned: Vec::new(),
            parameters,
            result_unproven: false,
            through: Vec::new(),
            unparametric: false,
        })));
    }

    /// Watches the calls through the function-typed parameters of the
    /// generic function being checked: `parameters`, by name, with the types
    /// each takes.
    pub(super) fn watch_function_parameters(&self, parameters: Vec<(String, Vec<Type>)>) {
        if let Some(function) = &self.function {
            function.borrow_mut().through = parameters;
        }
    }

    /// The types the function-typed parameter `name` of the generic function
    /// being checked takes, when it is one.
    pub(super) fn through_parameter(&self, name: &str) -> Option<Vec<Type>> {
        let function = self.function.as_ref()?;
        let function = function.borrow();
        function
            .through
            .iter()
            .find(|(parameter, _)| parameter == name)
            .map(|(_, types)| types.clone())
    }

    /// Notes that the function being checked passed one of its
    /// function-typed parameters a value not exactly of the type it takes.
    pub(super) fn note_unparametric(&self) {
        if let Some(function) = &self.function {
            function.borrow_mut().unparametric = true;
        }
    }

    /// Whether the function being checked passed a function-typed parameter
    /// anything but what it takes.
    pub(super) fn unparametric(&self) -> bool {
        self.function
            .as_ref()
            .is_some_and(|function| function.borrow().unparametric)
    }

    /// The entry check of the parameter at `index` of the function being
    /// checked, if it has one: its name, type, and whether it was inferred.
    pub(super) fn parameter_check(&self, index: usize) -> Option<(String, Type, bool)> {
        self.function
            .as_ref()
            .and_then(|function| function.borrow().parameters.get(index).cloned().flatten())
    }

    /// The declared result of the function being checked, if any.
    pub(super) fn expected_return(&self) -> Option<Type> {
        self.function
            .as_ref()
            .and_then(|function| function.borrow().expected.clone())
    }

    /// Notes that the function being checked may complete with a value of
    /// type `actual`, where its declared result is `expected`: when a part
    /// of it the checker could not prove reaches a type parameter, which a
    /// runtime check cannot test, the result is unproven.
    pub(super) fn note_result(&self, actual: &Type, expected: &Type) {
        if let Some(function) = &self.function
            && !actual.is_exactly_assignable_to(expected)
        {
            function.borrow_mut().result_unproven = true;
        }
    }

    /// Whether the function being checked has an unproven result.
    pub(super) fn result_unproven(&self) -> bool {
        self.function
            .as_ref()
            .is_some_and(|function| function.borrow().result_unproven)
    }

    /// Records that the function literal at `span` has an unproven result.
    pub(super) fn record_unproven_result(&self, span: SourceSpan) {
        if !self.in_trial() {
            self.records.borrow_mut().unproven_results.insert(span);
        }
    }

    /// Whether the function literal at `span` has an unproven result.
    pub(super) fn has_unproven_result(&self, span: &SourceSpan) -> bool {
        self.records.borrow().unproven_results.contains(span)
    }

    /// Records that the generic call at `site`, of the callee `identity`,
    /// checks its result against `result` if the callee's result is
    /// unproven, which may be known only once the module is checked.
    pub(super) fn record_conditional_result(
        &self,
        site: SourceSpan,
        result: Type,
        identity: CallableIdentity,
    ) {
        if !self.in_trial() {
            self.records
                .borrow_mut()
                .conditional_results
                .push((site, result, identity));
        }
    }

    /// `expected` as a runtime check tests it, with each enum that has
    /// payload cases as the union of the enum, for its fieldless cases, and
    /// its payload cases' structs, which is what its values are.
    fn runtime_check_type(&self, expected: &Type) -> Type {
        let part = |value: &Type| self.runtime_check_type(value);
        match expected {
            // Type arguments are expanded too: a generic struct's check
            // takes its fields' types from them.
            Type::Enum(identity, arguments) => {
                let arguments = arguments.iter().map(part).collect::<Vec<_>>();
                let enum_type = Type::Enum(identity.clone(), arguments.clone());
                let payloads = self.enum_payloads(identity);
                if payloads.is_empty() {
                    return enum_type;
                }
                Type::Union(
                    std::iter::once(enum_type)
                        .chain(payloads.iter().map(|case| {
                            Type::Struct(Some(identity.enum_case(case)), arguments.clone())
                        }))
                        .collect(),
                )
            }
            Type::Struct(identity, arguments) => {
                Type::Struct(identity.clone(), arguments.iter().map(part).collect())
            }
            Type::List(Some(element)) => Type::List(Some(Box::new(part(element)))),
            Type::Map(Some((key, value))) => {
                Type::Map(Some((Box::new(part(key)), Box::new(part(value)))))
            }
            Type::Union(members) => Type::Union(members.iter().map(part).collect()),
            Type::Tuple(elements) => Type::Tuple(elements.iter().map(part).collect()),
            Type::Record { fields, map } => Type::Record {
                fields: fields
                    .iter()
                    .map(|(name, field)| (name.clone(), part(field)))
                    .collect::<Vec<_>>()
                    .into(),
                map: *map,
            },
            other => other.clone(),
        }
    }

    /// Records the type a `return` in the current function produces.
    pub(super) fn record_return(&self, value_type: Type) {
        if let Some(function) = &self.function {
            function.borrow_mut().returned.push(value_type);
        }
    }

    /// The types every `return` in the current function produced.
    pub(super) fn returned_types(&self) -> Vec<Type> {
        self.function
            .as_ref()
            .map(|function| function.borrow().returned.clone())
            .unwrap_or_default()
    }

    /// Installs the contract's export `name`, a function or a schema, as an
    /// implicit builtin, so calls to it and `struct<name>` constraints check
    /// against its declaration.
    pub(super) fn install_implicit_export(&mut self, module: &ModuleSnapshot, name: &str) {
        let Some(binding) = module
            .exports
            .get(name)
            .filter(|binding| !binding.callables.is_empty() || binding.schema_identity.is_some())
        else {
            return;
        };
        let mut binding = binding.clone();
        binding.implicit = true;
        Rc::make_mut(
            self.scopes
                .first_mut()
                .expect("a semantic environment always has a root scope"),
        )
        .insert(name.into(), Rc::new(binding));
    }

    pub(super) fn import_snapshot(&self, name: &str) -> Option<ModuleSnapshot> {
        self.imports.get(name).cloned()
    }

    pub(super) fn enter_scope(&mut self) {
        self.scopes.push(Rc::default());
        self.type_scopes.push(Rc::default());
    }

    /// The innermost scope, to change.
    fn innermost_mut(&mut self) -> &mut Scope {
        Rc::make_mut(
            self.scopes
                .last_mut()
                .expect("a semantic environment always has a scope"),
        )
    }

    /// The innermost scope's binding of `name`, to change, when it has one.
    fn innermost_binding_mut(&mut self, name: &str) -> Option<&mut SemanticBinding> {
        if !self.scopes.last()?.contains_key(name) {
            return None;
        }
        self.innermost_mut().get_mut(name).map(Rc::make_mut)
    }

    #[cfg(test)]
    pub(super) fn leave_scope(&mut self) {
        debug_assert!(self.scopes.len() > 1);
        self.scopes.pop();
        self.type_scopes.pop();
    }

    pub(super) fn declare(&mut self, name: String, binding: SemanticBinding) {
        self.innermost_mut().insert(name, Rc::new(binding));
    }

    pub(super) fn declare_type(&mut self, name: String, member: TypeMember) {
        Rc::make_mut(
            self.type_scopes
                .last_mut()
                .expect("a semantic environment always has a type scope"),
        )
        .insert(name, member);
    }

    pub(super) fn declare_callable(
        &mut self,
        name: String,
        signature: CallableSignature,
        span: &SourceSpan,
    ) -> Result<(), super::SourceError> {
        let scope = self.innermost_mut();
        let Some(existing) = scope.get_mut(&name).map(Rc::make_mut) else {
            scope.insert(name, Rc::new(SemanticBinding::callable(signature)));
            return Ok(());
        };
        if existing.callables.is_empty() || existing.implicit {
            *existing = SemanticBinding::callable(signature);
            return Ok(());
        }
        if existing
            .callables
            .iter()
            .any(|candidate| candidate.has_same_input(&signature))
        {
            return Err(super::SourceError::semantic(
                super::SourceErrorCode::DuplicateSignature,
                format!("duplicate callable signature for `{name}`"),
                span.clone(),
            ));
        }
        existing.callables.push(signature);
        Ok(())
    }

    pub(super) fn update_callable_result(
        &mut self,
        name: &str,
        identity: &CallableIdentity,
        result: Type,
        result_unproven: bool,
        parametric: bool,
    ) {
        let Some(binding) = self.innermost_binding_mut(name) else {
            return;
        };
        if let Some(signature) = binding
            .callables
            .iter_mut()
            .find(|signature| signature.identity() == *identity)
        {
            signature.result = result;
            signature.result_unproven = Some(result_unproven);
            signature.parametric = parametric;
            binding.value_type = function_value_type(signature);
        }
    }

    /// Makes the named binding a `var` whose declared type is its current
    /// one, except that a `nil` initializer declares `nil` with the type of
    /// the first assignment a trial check found, or else `any|nil`: `nil`
    /// there means no value yet, not a value that is always `nil`.
    pub(super) fn declare_mutable(&mut self, name: &str, declaration: &SourceSpan) {
        let in_trial = self.in_trial();
        let site = (declaration.clone(), name.to_string());
        let inferred = self.nil_starts.borrow().inferred.get(&site).cloned();
        if let Some(binding) = self.innermost_binding_mut(name) {
            if binding.value_type == Type::Nil {
                let assignments = inferred.unwrap_or_default();
                binding.value_type = if assignments.is_empty() && in_trial {
                    Type::Unknown
                } else if assignments.is_empty() {
                    Type::universal()
                } else {
                    Type::union(
                        std::iter::once(Type::Nil)
                            .chain(assignments.iter().map(|(_, value_type)| value_type.clone())),
                    )
                };
                binding.nil_start = Some(NilStart { site, assignments });
            }
            // A `var`'s later values need not keep its first value's length
            // or keys.
            binding.value_type = binding.value_type.widen_literals();
            binding.declared = Some(binding.value_type.clone());
        }
    }

    /// Forgets what assignments and nil tests established about every `var`
    /// in scope, as a nested function must: it may run after any later
    /// assignment.
    pub(super) fn forget_mutable_facts(&mut self) {
        let established = |binding: &SemanticBinding| {
            binding
                .declared
                .as_ref()
                .is_some_and(|declared| *declared != binding.value_type)
        };
        for scope in &mut self.scopes {
            if !scope.values().any(|binding| established(binding)) {
                continue;
            }
            for binding in Rc::make_mut(scope).values_mut() {
                if established(binding) {
                    let binding = Rc::make_mut(binding);
                    binding.value_type = binding
                        .declared
                        .clone()
                        .expect("an established fact is about a declared var");
                }
            }
        }
    }

    /// The binding of `name` in the innermost scope only.
    pub(super) fn lookup_in_scope(&self, name: &str) -> Option<&SemanticBinding> {
        self.scopes.last()?.get(name).map(Rc::as_ref)
    }

    pub(super) fn lookup(&self, name: &str) -> Option<&SemanticBinding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .map(Rc::as_ref)
    }

    /// The binding of `name`, to change: its scope and the binding are
    /// copied first when another environment shares them.
    pub(super) fn lookup_mut(&mut self, name: &str) -> Option<&mut SemanticBinding> {
        let scope = self
            .scopes
            .iter_mut()
            .rev()
            .find(|scope| scope.contains_key(name))?;
        Rc::make_mut(scope).get_mut(name).map(Rc::make_mut)
    }

    pub(super) fn schema_by_identity(&self, identity: &SchemaIdentity) -> Option<SemanticBinding> {
        self.scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.values())
            .find_map(|binding| find_schema_binding(binding, identity))
    }

    pub(super) fn resource_type(&self, name: &str) -> Option<ResourceIdentity> {
        if let Some((module, type_name)) = name.split_once('.') {
            return self
                .lookup(module)
                .filter(|binding| binding.module_binding)
                .and_then(|binding| binding.type_members.get(type_name))
                .and_then(|member| match member {
                    TypeMember::Resource(identity) => Some(identity.clone()),
                    TypeMember::Enum { .. }
                    | TypeMember::Alias(_)
                    | TypeMember::GenericAlias { .. } => None,
                });
        }
        self.type_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .and_then(|member| match member {
                TypeMember::Resource(identity) => Some(identity.clone()),
                TypeMember::Enum { .. }
                | TypeMember::Alias(_)
                | TypeMember::GenericAlias { .. } => None,
            })
    }

    pub(super) fn type_member(&self, name: &str) -> Option<&TypeMember> {
        if let Some((module, type_name)) = name.split_once('.') {
            return self
                .lookup(module)
                .filter(|binding| binding.module_binding)
                .and_then(|binding| binding.type_members.get(type_name));
        }
        self.type_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
    }

    pub(super) fn enum_cases(&self, identity: &EnumIdentity) -> Option<Vec<String>> {
        self.enum_member(identity).map(|(cases, _)| cases)
    }

    /// The cases of the enum `identity` names that carry a payload.
    pub(super) fn enum_payloads(&self, identity: &EnumIdentity) -> Vec<String> {
        self.enum_member(identity)
            .map(|(_, payloads)| payloads)
            .unwrap_or_default()
    }

    fn enum_member(&self, identity: &EnumIdentity) -> Option<(Vec<String>, Vec<String>)> {
        self.type_scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.values())
            .chain(
                self.scopes
                    .iter()
                    .rev()
                    .flat_map(|scope| scope.values())
                    .flat_map(|binding| binding.type_members.values()),
            )
            .find_map(|member| match member {
                TypeMember::Enum {
                    identity: candidate,
                    cases,
                    payloads,
                    ..
                } if candidate == identity => Some((cases.clone(), payloads.clone())),
                TypeMember::Resource(_)
                | TypeMember::Enum { .. }
                | TypeMember::Alias(_)
                | TypeMember::GenericAlias { .. } => None,
            })
    }

    pub(super) fn resolve_resource_type(
        &self,
        name: &str,
        span: &SourceSpan,
    ) -> Result<ResourceIdentity, super::SourceError> {
        if let Some((module, type_name)) = name.split_once('.') {
            let binding = self.lookup(module).ok_or_else(|| {
                super::SourceError::semantic(
                    super::SourceErrorCode::UnknownName,
                    format!("unknown module binding `{module}`"),
                    span.clone(),
                )
            })?;
            if let Some(unavailable) = &binding.unavailable_module {
                return Err(super::SourceError::semantic(
                    super::SourceErrorCode::UnknownType,
                    format!(
                        "`{name}` cannot be resolved: module `{unavailable}`, imported as \
                         `{module}`, has no types here, because it does not compile or it \
                         imports this module"
                    ),
                    span.clone(),
                ));
            }
            if !binding.module_binding {
                return Err(super::SourceError::semantic(
                    super::SourceErrorCode::UnknownType,
                    format!("type prefix `{module}` is not a module binding"),
                    span.clone(),
                ));
            }
            return binding
                .type_members
                .get(type_name)
                .and_then(|member| match member {
                    TypeMember::Resource(identity) => Some(identity.clone()),
                    TypeMember::Enum { .. }
                    | TypeMember::Alias(_)
                    | TypeMember::GenericAlias { .. } => None,
                })
                .ok_or_else(|| {
                    super::SourceError::semantic(
                        super::SourceErrorCode::UnknownType,
                        format!("unknown type `{name}`"),
                        span.clone(),
                    )
                });
        }
        self.resource_type(name).ok_or_else(|| {
            let message = if self
                .lookup(name)
                .is_some_and(|binding| binding.schema_identity.is_some())
            {
                format!("`{name}` is a struct schema, not a type; write `struct<{name}>`")
            } else {
                format!("unknown type `{name}`")
            };
            super::SourceError::semantic(super::SourceErrorCode::UnknownType, message, span.clone())
        })
    }

    /// The types bindings may have after two branches, `left` and `right`,
    /// rejoin: each binding's type is the union of its types in them, since
    /// either may have run. A binding a branch assigned is not left at its
    /// type from before the branch, which that assignment may have changed.
    pub(super) fn merge_compatible_types(&mut self, left: &Self, right: &Self) {
        for (index, scope) in self.scopes.iter_mut().enumerate() {
            let (Some(left_scope), Some(right_scope)) =
                (left.scopes.get(index), right.scopes.get(index))
            else {
                continue;
            };
            // A scope neither branch changed is still this one.
            if Rc::ptr_eq(scope, left_scope) && Rc::ptr_eq(scope, right_scope) {
                continue;
            }
            let merged = scope
                .iter()
                .filter_map(|(name, binding)| {
                    let (Some(left), Some(right)) = (left_scope.get(name), right_scope.get(name))
                    else {
                        return None;
                    };
                    if Rc::ptr_eq(binding, left) && Rc::ptr_eq(binding, right) {
                        return None;
                    }
                    let value_type = if left.value_type == right.value_type {
                        left.value_type.clone()
                    } else {
                        Type::union([left.value_type.clone(), right.value_type.clone()])
                    };
                    // A member fact holds after the branches only when each
                    // one that continues established it.
                    let member_facts = left
                        .member_facts
                        .iter()
                        .filter(|(key, value_type)| {
                            right.member_facts.get(*key) == Some(*value_type)
                        })
                        .map(|(key, value_type)| (key.clone(), value_type.clone()))
                        .collect::<HashMap<_, _>>();
                    (binding.value_type != value_type || binding.member_facts != member_facts)
                        .then(|| (name.clone(), value_type, member_facts))
                })
                .collect::<Vec<_>>();
            if merged.is_empty() {
                continue;
            }
            let scope = Rc::make_mut(scope);
            for (name, value_type, member_facts) in merged {
                let binding = Rc::make_mut(scope.get_mut(&name).expect("merged from this scope"));
                binding.value_type = value_type;
                binding.member_facts = member_facts;
            }
        }
    }

    /// Merge the environments that can reach a control-flow continuation.
    /// Returns whether at least one input environment survives.
    pub(super) fn merge_reachable_types(
        &mut self,
        left: Option<&Self>,
        right: Option<&Self>,
    ) -> bool {
        match (left, right) {
            (Some(left), Some(right)) => {
                self.merge_compatible_types(left, right);
                true
            }
            (Some(environment), None) | (None, Some(environment)) => {
                self.merge_compatible_types(environment, environment);
                true
            }
            (None, None) => false,
        }
    }

    pub(super) fn import(&self, name: &str) -> Option<&ModuleSnapshot> {
        self.imports.get(name)
    }

    /// Notes that the parameters at `parameters`' positions of the function
    /// literal at `span` range over the keys of those maps.
    pub(super) fn record_key_parameters(
        &self,
        span: SourceSpan,
        parameters: Vec<(usize, KeyPath)>,
    ) {
        self.records
            .borrow_mut()
            .key_parameters
            .insert(span, parameters);
    }

    /// The map whose keys parameter `index` of the function literal at
    /// `span` ranges over, if it does.
    pub(super) fn key_parameter(&self, span: &SourceSpan, index: usize) -> Option<KeyPath> {
        self.records
            .borrow()
            .key_parameters
            .get(span)?
            .iter()
            .find(|(position, _)| *position == index)
            .map(|(_, path)| path.clone())
    }

    /// Notes whether the generic function literal at `span` was found
    /// parametric this time it was checked.
    pub(super) fn record_parametric(&self, span: SourceSpan, parametric: bool) {
        let mut records = self.records.borrow_mut();
        let entry = records.parametric.entry(span).or_insert(true);
        *entry &= parametric;
    }

    /// Whether the function literal at `span` was found parametric each
    /// time it was checked.
    pub(super) fn is_parametric(&self, span: &SourceSpan) -> bool {
        self.records
            .borrow()
            .parametric
            .get(span)
            .copied()
            .unwrap_or(false)
    }

    /// Notes whether the function literal at `span`, given to a parametric
    /// function, needs no checks of its parameters this time it is checked.
    pub(super) fn record_trusted_closure(&self, span: SourceSpan, trusted: bool) {
        let mut records = self.records.borrow_mut();
        let entry = records.trusted_closures.entry(span).or_insert(true);
        *entry &= trusted;
    }

    /// Notes whether the named function at `span`, given to a parametric
    /// function that passes it `arity` values, would be trusted as a literal
    /// is, this time it is checked; `None` when not.
    pub(super) fn record_trusted_reference(&self, span: SourceSpan, arity: Option<usize>) {
        let mut records = self.records.borrow_mut();
        let entry = records.trusted_references.entry(span).or_insert(arity);
        if *entry != arity {
            *entry = None;
        }
    }

    /// Notes whether the call at `span` was proven, this time it is checked,
    /// to pass every one of its callee's entry checks.
    pub(super) fn record_proven_call(&self, span: SourceSpan, proven: bool) {
        let mut records = self.records.borrow_mut();
        let entry = records.proven_calls.entry(span).or_insert(true);
        *entry &= proven;
    }

    pub(super) fn record_selected_call(&self, span: SourceSpan, identity: CallableIdentity) {
        self.records
            .borrow_mut()
            .selected_calls
            .insert(span, identity);
    }

    pub(super) fn record_function(&self, span: SourceSpan, identity: CallableIdentity) {
        self.records
            .borrow_mut()
            .function_identities
            .insert(span, identity);
    }

    pub(super) fn record_foreign(
        &self,
        span: SourceSpan,
        identity: CallableIdentity,
        resource_signature: ForeignResourceSignature,
    ) {
        let mut records = self.records.borrow_mut();
        records.foreign_identities.insert(span.clone(), identity);
        records
            .foreign_resource_signatures
            .insert(span, resource_signature);
    }

    pub(super) fn record_match_constraints(
        &self,
        span: SourceSpan,
        constraints: Vec<Option<Type>>,
    ) {
        self.records
            .borrow_mut()
            .match_constraints
            .insert(span, constraints);
    }

    /// Records the type of the arguments the subject-less `match` at `span`
    /// matches, as a function's body.
    pub(super) fn record_implicit_subject(&self, span: SourceSpan, value_type: Type) {
        self.records
            .borrow_mut()
            .implicit_subjects
            .insert(span, value_type);
    }

    /// The type of the arguments the subject-less `match` at `span` matches.
    pub(super) fn implicit_subject(&self, span: &SourceSpan) -> Option<Type> {
        self.records.borrow().implicit_subjects.get(span).cloned()
    }

    /// Records the type an annotation gives the result of the call at `span`.
    pub(super) fn expect_result(&self, span: SourceSpan, value_type: Type) {
        self.records
            .borrow_mut()
            .expected_results
            .insert(span, value_type);
    }

    /// The type an annotation gives the result of the call at `span`.
    pub(super) fn expected_result(&self, span: &SourceSpan) -> Option<Type> {
        self.records.borrow().expected_results.get(span).cloned()
    }

    /// Records the parameter types expected of the function literal at
    /// `span`, as an argument.
    pub(super) fn expect_parameters(&self, span: SourceSpan, types: Vec<Type>) {
        self.records
            .borrow_mut()
            .expected_parameters
            .insert(span, types);
    }

    /// Records the result expected of the function literal at `span`.
    pub(super) fn expect_literal_result(&self, span: SourceSpan, result: Type) {
        self.records
            .borrow_mut()
            .expected_literal_results
            .insert(span, result);
    }

    /// The result expected of the function literal at `span`, if known.
    pub(super) fn expected_literal_result(&self, span: &SourceSpan) -> Option<Type> {
        self.records
            .borrow()
            .expected_literal_results
            .get(span)
            .cloned()
    }

    /// The parameter types expected of the function literal at `span`.
    pub(super) fn expected_parameters(&self, span: &SourceSpan) -> Option<Vec<Type>> {
        self.records.borrow().expected_parameters.get(span).cloned()
    }

    /// The type last recorded for the expression at `span`.
    pub(super) fn recorded_expression_type(&self, span: &SourceSpan) -> Option<Type> {
        self.records.borrow().expression_types.get(span).cloned()
    }

    pub(super) fn record_expression_type(&self, span: SourceSpan, value_type: Type) {
        self.records
            .borrow_mut()
            .expression_types
            .insert(span, value_type);
    }

    /// Records that the site at `site` checks its value, of type `actual`,
    /// against `expected` when it runs, unless the checker proved it
    /// conforms. A trial check records nothing: the check that follows it
    /// does.
    pub(super) fn record_boundary_check(
        &self,
        site: &SourceSpan,
        actual: &Type,
        expected: &Type,
        place: CheckPlace,
    ) {
        if self.in_trial() || actual.is_proven_assignable_to(expected) {
            return;
        }
        let check = BoundaryCheck {
            expected: expected.clone(),
            place,
            runtime: self.runtime_check_type(expected),
        };
        let mut records = self.records.borrow_mut();
        let checks = records.boundary_checks.entry(site.clone()).or_default();
        if !checks.contains(&check) {
            checks.push(check);
        }
    }

    /// Records the names a `{*}` declaration defines, or, with `None`, that
    /// they are not known until it runs.
    pub(super) fn record_selection<'a>(&self, names: Option<impl Iterator<Item = &'a String>>) {
        let mut records = self.records.borrow_mut();
        match names {
            Some(names) => records.selected_globals.extend(names.cloned()),
            None => records.unknown_selection = true,
        }
    }

    pub(super) fn analysis(&self, snapshot: ModuleSnapshot) -> SemanticAnalysis {
        let records = self.records.borrow();
        // A generic call checks its result when its callee, declared here,
        // turned out to have an unproven result.
        let unproven = records
            .unproven_results
            .iter()
            .filter_map(|span| records.function_identities.get(span))
            .collect::<Vec<_>>();
        let mut boundary_checks = records.boundary_checks.clone();
        for (site, result, identity) in &records.conditional_results {
            if unproven.contains(&identity) {
                let check = BoundaryCheck {
                    expected: result.clone(),
                    place: CheckPlace::CallResult,
                    runtime: result.clone(),
                };
                let checks = boundary_checks.entry(site.clone()).or_default();
                if !checks.contains(&check) {
                    checks.push(check);
                }
            }
        }
        SemanticAnalysis {
            snapshot,
            session_snapshot: self.session_snapshot(),
            selected_calls: records.selected_calls.clone(),
            proven_calls: records
                .proven_calls
                .iter()
                .filter(|(_, proven)| **proven)
                .map(|(span, _)| span.clone())
                .collect(),
            trusted_closures: records
                .trusted_closures
                .iter()
                .filter(|(_, trusted)| **trusted)
                .map(|(span, _)| span.clone())
                .collect(),
            trusted_references: records
                .trusted_references
                .iter()
                .filter_map(|(span, arity)| arity.map(|arity| (span.clone(), arity)))
                .collect(),
            function_identities: records.function_identities.clone(),
            foreign_identities: records.foreign_identities.clone(),
            foreign_resource_signatures: records.foreign_resource_signatures.clone(),
            match_constraints: records.match_constraints.clone(),
            expression_types: records.expression_types.clone(),
            boundary_checks,
            selected_globals: (!records.unknown_selection)
                .then(|| records.selected_globals.clone()),
        }
    }

    fn session_snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            bindings: self
                .scopes
                .first()
                .map(|scope| {
                    scope
                        .iter()
                        .map(|(name, binding)| (name.clone(), SemanticBinding::clone(binding)))
                        .collect()
                })
                .unwrap_or_default(),
            types: self
                .type_scopes
                .first()
                .map(|scope| HashMap::clone(scope))
                .unwrap_or_default(),
        }
    }
}

fn find_schema_binding(
    binding: &SemanticBinding,
    identity: &SchemaIdentity,
) -> Option<SemanticBinding> {
    if binding
        .schema_identity
        .as_ref()
        .is_some_and(|schema| schema == identity)
    {
        return Some(binding.clone());
    }
    binding
        .members
        .values()
        .chain(&binding.schemas)
        .find_map(|member| find_schema_binding(member, identity))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(input: Type, result: Type) -> CallableSignature {
        CallableSignature {
            generic_arity: 0,
            type_parameter_names: Vec::new(),
            parameters: vec![CallableParameter {
                label: Some("value".into()),
                value_type: input,
                has_default: false,
                variadic: false,
                inferred: false,
            }],
            result,
            inferred_alternatives: Vec::new(),
            result_unproven: Some(false),
            parametric: false,
        }
    }

    #[test]
    fn lexical_bindings_shadow_and_restore_callable_sets() {
        let mut environment = Environment::new();
        environment.declare(
            "render".into(),
            SemanticBinding::callable(signature(Type::Str, Type::Str)),
        );
        environment.enter_scope();
        environment.declare("render".into(), SemanticBinding::value(Type::Num));
        assert!(environment.lookup("render").unwrap().callables.is_empty());
        environment.leave_scope();
        assert_eq!(environment.lookup("render").unwrap().callables.len(), 1);
    }

    #[test]
    fn result_types_do_not_participate_in_callable_identity() {
        let left = signature(Type::Str, Type::Str);
        let right = signature(Type::Str, Type::Num);
        assert!(left.has_same_input(&right));
    }
}
