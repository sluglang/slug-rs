use crate::sync::Rc;
use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};

use crate::Value;

/// A source position attached to an instruction for language diagnostics.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct SourceSpan {
    pub path: Arc<str>,
    pub line: u32,
    pub column: u32,
}

impl SourceSpan {
    #[must_use]
    pub fn new(path: impl Into<String>, line: u32, column: u32) -> Self {
        Self {
            path: Arc::from(path.into()),
            line,
            column,
        }
    }
}

/// Private source-path table index used by bytecode span metadata.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceId(pub(crate) u32);

impl SourceId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Private source-span table index used by bytecode instructions.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SpanId(pub(crate) u32);

impl SpanId {
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

macro_rules! metadata_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub(crate) u32);

        impl $name {
            #[must_use]
            pub const fn new(index: u32) -> Self {
                Self(index)
            }

            /// The index `new` was given.
            #[must_use]
            pub const fn get(self) -> u32 {
                self.0
            }

            pub(crate) fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

metadata_id!(GlobalNameId);
metadata_id!(CaptureListId);
metadata_id!(SchemaFieldsId);
metadata_id!(StructFieldsId);
metadata_id!(MatchPatternId);
metadata_id!(InterpolationId);
metadata_id!(ListSpreadId);
metadata_id!(CallArgumentsId);
metadata_id!(SelectedCallId);
metadata_id!(SelectCasesId);

/// Opaque input identity used only for private runtime callable selection.
///
/// A thin `Rc<String>` rather than `Rc<str>` keeps it one pointer wide.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallableIdentity(Rc<String>);

impl CallableIdentity {
    #[doc(hidden)]
    #[must_use]
    pub fn new(canonical_input: impl Into<String>) -> Self {
        Self(Rc::new(canonical_input.into()))
    }

    #[allow(dead_code)] // Consumed by the in-progress `.cslug` encoder.
    pub(crate) fn cslug_text(&self) -> &str {
        &self.0
    }
}

/// Resource positions retained for runtime validation of a foreign declaration.
#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct ForeignResourceSignature {
    parameters: Vec<Option<(Option<String>, String)>>,
    result: Option<(Option<String>, String)>,
    /// The kinds the result's annotation accepts, as `check_kinds` bits,
    /// and the annotated type, which a failed check names.
    result_kinds: Option<(u32, String)>,
}

impl ForeignResourceSignature {
    #[doc(hidden)]
    #[must_use]
    pub fn new(
        parameters: Vec<Option<(Option<String>, String)>>,
        result: Option<(Option<String>, String)>,
    ) -> Self {
        Self {
            parameters,
            result,
            result_kinds: None,
        }
    }

    /// This signature, with its result checked to be of one of `kinds`,
    /// `check_kinds` bits, as the result's annotation `expected` says.
    #[doc(hidden)]
    #[must_use]
    pub fn with_result_kinds(mut self, kinds: u32, expected: String) -> Self {
        self.result_kinds = Some((kinds, expected));
        self
    }

    pub(crate) fn result_kinds(&self) -> Option<(u32, &str)> {
        self.result_kinds
            .as_ref()
            .map(|(kinds, expected)| (*kinds, expected.as_str()))
    }

    pub(crate) fn parameter_identity(&self, index: usize) -> Option<(Option<&str>, &str)> {
        self.parameters
            .get(index)
            .and_then(Option::as_ref)
            .map(|(module, name)| (module.as_deref(), name.as_str()))
    }

    #[allow(dead_code)] // Consumed by the in-progress `.cslug` encoder.
    pub(crate) fn cslug_parameter_count(&self) -> usize {
        self.parameters.len()
    }

    pub(crate) fn cslug_parameter_identities(
        &self,
    ) -> impl Iterator<Item = Option<(Option<&str>, &str)>> {
        self.parameters.iter().map(|identity| {
            identity
                .as_ref()
                .map(|(module, name)| (module.as_deref(), name.as_str()))
        })
    }

    pub(crate) fn cslug_result_identity(&self) -> Option<(Option<&str>, &str)> {
        self.result_identity()
    }

    pub(crate) fn result_identity(&self) -> Option<(Option<&str>, &str)> {
        self.result
            .as_ref()
            .map(|(module, name)| (module.as_deref(), name.as_str()))
    }
}

/// A literal embedded in a bytecode chunk.
#[derive(Clone, Debug)]
pub enum Constant {
    Value(Value),
    Function(usize),
}

/// The enclosing slot from which a closure captures a value.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Capture {
    Local(usize),
    Capture(usize),
}

/// The subset of source patterns lowered by the current compiler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatchRest {
    None,
    Discard,
    Binding,
}

#[derive(Clone, Debug)]
pub enum MatchMapKey {
    String(String),
    Operand(usize),
}

#[derive(Clone, Debug)]
pub struct SchemaField {
    pub name: String,
    pub has_default: bool,
    /// The kinds the field's annotation accepts, as `check_kinds` bits, and
    /// the annotated type, which a failed check names: every value given
    /// the field, by a construction or a copy, is checked to be of one.
    pub check: Option<(u32, String)>,
}

/// Private callable metadata used by source-call binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterSignature {
    pub name: String,
    pub has_default: bool,
    pub variadic: bool,
    /// The kinds the parameter's entry check accepts, as `check_kinds` bits,
    /// when it has one: a call to an overload set the compiler could not
    /// resolve takes a member whose parameters accept its arguments.
    pub kinds: Option<u32>,
}

/// Retained evaluated tag metadata for a top-level source declaration.
#[derive(Clone, Debug)]
pub struct ModuleTag {
    pub name: String,
    pub arguments: Vec<Value>,
}

/// The binding a function captures in one capture slot: whether it is a
/// `var`, which a fork cannot share with its spawner, and if so its name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureDetail {
    pub name: String,
    pub mutable: bool,
}

/// Retained source metadata for a top-level declaration.
// Each flag is a separate fact a source declaration states (`var`, `task`,
// `export`, `foreign`), and `describe` reports them as such.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug)]
pub struct ModuleDeclaration {
    pub bindings: Vec<String>,
    pub mutable: bool,
    /// Whether a mutable declaration is a `task var`, which every task owns a
    /// copy of.
    pub task: bool,
    pub exported: bool,
    /// Whether this declaration must be supplied by the module-qualified host registry.
    pub foreign: bool,
    /// The inclusive declared call-arity range for a foreign binding. `None`
    /// denotes a variadic declaration.
    pub foreign_arity: Option<(usize, Option<usize>)>,
    /// The source-level nominal resource type declared by this metadata entry.
    pub resource_type: Option<String>,
    /// Private canonical callable identity for a resolved foreign binding.
    #[doc(hidden)]
    pub foreign_callable_identity: Option<CallableIdentity>,
    /// Resource positions that require validation when invoking this foreign binding.
    #[doc(hidden)]
    pub foreign_resource_signature: Option<ForeignResourceSignature>,
    pub documentation: Option<String>,
    pub tags: Vec<ModuleTag>,
}

/// The subset of source patterns lowered by the current compiler.
#[derive(Clone, Debug)]
pub enum MatchPattern {
    Literal(Value),
    /// A qualified enum case evaluated before attempting the pattern.
    Enum(usize),
    Wildcard,
    Binding,
    Pinned(usize),
    At(Box<MatchPattern>),
    Alternatives(Vec<MatchPattern>),
    List {
        items: Vec<MatchPattern>,
        rest: MatchRest,
    },
    Map {
        entries: Vec<(MatchMapKey, MatchPattern)>,
        rest: MatchRest,
        exact: bool,
    },
    Constrained {
        pattern: Box<MatchPattern>,
        constraint: MatchType,
    },
}

/// Private runtime-checkable type form used by source match patterns.
#[derive(Clone, Debug)]
pub enum MatchType {
    Any,
    Nil,
    Bool,
    Num,
    Str,
    Bytes,
    Resource {
        module: String,
        name: String,
    },
    Enum {
        module: String,
        name: String,
    },
    List(Option<Box<MatchType>>),
    Map(Option<(Box<MatchType>, Box<MatchType>)>),
    Function,
    Task,
    Channel,
    Schema,
    Struct(Option<usize>),
    Union(Vec<MatchType>),
    /// A record type: a map whose value at each string key, a missing key
    /// reading `nil`, or a struct whose field of each name, matches the
    /// field's type.
    Record(Vec<(String, MatchType)>),
}

/// One source-order case consumed by the private select instruction.
#[derive(Clone, Debug)]
pub enum SelectCase {
    Receive { has_handler: bool },
    Send { has_handler: bool },
    After { has_handler: bool },
    Await { has_handler: bool },
    Default { has_handler: bool },
}
