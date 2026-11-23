//! Numeric kernels with their numbers in registers: a prototype.
//!
//! A function whose entry checks prove each parameter a number, and whose
//! body only computes with numbers and booleans, branches, and returns or
//! recurs, gets a version for each combination of its parameters being
//! integers or floats that its `recur`s keep. A version keeps every value
//! in a register, with no tag, and writes a `Value` only for its result.
//! The entry, past the checks, takes the version the arguments' tags name.
//!
//! What a version cannot compute as the general code would (an overflow,
//! a division with a remainder, which Slug makes a float, a result that is
//! not finite) and a thread that needs attention at a `recur` write the
//! iteration's parameters back to their slots and run the iteration again
//! in the general code, past the checks. An iteration only computes, so
//! running it again gives the same result, or the same error.

use cranelift_codegen::ir::{
    Block as CodeBlock, InstBuilder, MemFlags, Value as CodeValue,
    condcodes::{FloatCC, IntCC},
    types,
};
use slug_vm::{BinaryOperator, Constant, Op, Value, check_kinds};

use super::{Lowering, Place, binary_operator};
use crate::{
    ir::{Edge, Function, Terminator, ValueId},
    kinds::Kinds,
    runtime::{RETURNED, layout, offsets},
};

/// How a version holds a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rep {
    Int,
    Float,
    Bool,
}

/// One version: how it holds each parameter, and each IR value.
pub(super) struct Version {
    params: Vec<Rep>,
    reps: Vec<Option<Rep>>,
}

/// The most parameters a kernel may have, as each doubles its versions.
const MAX_PARAMETERS: usize = 4;

/// The most versions a kernel gets, which bounds its code.
const MAX_VERSIONS: usize = 8;

/// The versions of `function` that keep their representations, if it is a
/// numeric kernel.
pub(super) fn versions(function: &Function, kinds: &Kinds) -> Vec<Version> {
    if !eligible(function, kinds) {
        return Vec::new();
    }
    (0..1usize << function.arity)
        .filter_map(|combination| {
            let params = (0..function.arity)
                .map(|index| {
                    if combination & (1 << index) == 0 {
                        Rep::Int
                    } else {
                        Rep::Float
                    }
                })
                .collect::<Vec<_>>();
            plan(function, params)
        })
        .take(MAX_VERSIONS)
        .collect()
}

/// Whether `function` is a numeric kernel: every parameter checked to be a
/// number, and nothing but arithmetic, comparisons, constants, branches,
/// returns, and `recur`s.
fn eligible(function: &Function, kinds: &Kinds) -> bool {
    let arity = function.arity;
    if arity == 0
        || arity > MAX_PARAMETERS
        || function.defers
        || function.cells.iter().any(|cell| *cell)
        || kinds.leading != arity
        || kinds
            .entry_checks
            .iter()
            .any(|check| *check != Some(check_kinds::NUM))
    {
        return false;
    }
    function.blocks.iter().enumerate().all(|(index, block)| {
        let terminator = match &block.terminator {
            Terminator::Jump(_) | Terminator::Branch { .. } | Terminator::Return(_) => true,
            Terminator::Recur(arguments) => arguments.len() == arity,
            Terminator::Choose { .. } | Terminator::Stop => false,
        };
        terminator
            && block
                .insts
                .iter()
                .enumerate()
                .all(|(position, inst)| match &inst.op {
                    Op::CheckKinds { kinds: checked, .. } => {
                        (index == 0 && position < kinds.leading)
                            || kinds.passes(inst.args[0], *checked)
                    }
                    Op::RecurPositional(_) | Op::True | Op::False | Op::Negate | Op::Not => true,
                    Op::Constant(constant) => matches!(
                        function.constants.get(*constant),
                        Some(Constant::Value(
                            Value::Int(_) | Value::Float(_) | Value::Bool(_)
                        ))
                    ),
                    op => binary_operator(op).is_some_and(supported),
                })
    })
}

/// Whether a version computes `operator`.
fn supported(operator: BinaryOperator) -> bool {
    use BinaryOperator as B;
    !matches!(operator, B::ShiftLeft | B::ShiftRight)
}

/// The representation of each value of `function` when its parameters are
/// `params`, if every one is known and its `recur`s keep `params`.
fn plan(function: &Function, params: Vec<Rep>) -> Option<Version> {
    let mut reps = vec![None; function.values.len()];
    for (param, rep) in function.blocks[0].params.iter().zip(&params) {
        reps[param.index()] = Some(*rep);
    }
    // Every edge but a `recur` goes forward, so passes in block order settle.
    for _ in 0..=function.blocks.len() {
        for (index, block) in function.blocks.iter().enumerate().skip(1) {
            for (position, param) in block.params.iter().enumerate() {
                let incoming = function
                    .blocks
                    .iter()
                    .flat_map(|from| from.terminator.edges())
                    .filter(|edge| edge.block.index() == index)
                    .map(|edge| reps[edge.args[position].index()])
                    .collect::<Vec<_>>();
                if let Some(Some(first)) = incoming.first()
                    && incoming.iter().all(|rep| *rep == Some(*first))
                {
                    reps[param.index()] = Some(*first);
                }
            }
        }
        for block in &function.blocks {
            for inst in &block.insts {
                if let Some(result) = inst.results.first() {
                    reps[result.index()] =
                        result_rep(function, &inst.op, &inst.args, &reps).ok()?;
                }
            }
        }
    }
    for block in &function.blocks {
        for inst in &block.insts {
            if inst
                .results
                .iter()
                .any(|result| reps[result.index()].is_none())
            {
                return None;
            }
        }
        for edge in block.terminator.edges() {
            let target = &function.blocks[edge.block.index()];
            for (arg, param) in edge.args.iter().zip(&target.params) {
                if reps[arg.index()].is_none() || reps[arg.index()] != reps[param.index()] {
                    return None;
                }
            }
        }
        let kept = match &block.terminator {
            Terminator::Recur(arguments) => arguments
                .iter()
                .zip(&params)
                .all(|(argument, rep)| reps[argument.index()] == Some(*rep)),
            Terminator::Branch { cond, .. } | Terminator::Return(cond) => {
                reps[cond.index()].is_some()
            }
            _ => true,
        };
        if !kept {
            return None;
        }
    }
    Some(Version { params, reps })
}

/// An instruction a version cannot compute.
struct Unsupported;

/// The representation of an instruction's result, `None` while an
/// operand's is not known yet.
fn result_rep(
    function: &Function,
    op: &Op,
    args: &[ValueId],
    reps: &[Option<Rep>],
) -> Result<Option<Rep>, Unsupported> {
    use BinaryOperator as B;
    let constant = |index: usize| match function.constants.get(index) {
        Some(Constant::Value(Value::Int(_))) => Ok(Rep::Int),
        Some(Constant::Value(Value::Float(_))) => Ok(Rep::Float),
        Some(Constant::Value(Value::Bool(_))) => Ok(Rep::Bool),
        _ => Err(Unsupported),
    };
    let arg = |index: usize| args.get(index).and_then(|arg| reps[arg.index()]);
    Ok(match op {
        Op::Constant(index) => Some(constant(*index)?),
        Op::True | Op::False => Some(Rep::Bool),
        Op::Not => match arg(0) {
            Some(Rep::Bool) => Some(Rep::Bool),
            None => None,
            // `not` of a number is `false`, which the general code gives.
            Some(_) => return Err(Unsupported),
        },
        Op::Negate => match arg(0) {
            Some(Rep::Bool) => return Err(Unsupported),
            rep => rep,
        },
        op => {
            let operator = binary_operator(op).ok_or(Unsupported)?;
            let left = arg(0);
            let right = match op {
                Op::LocalConstantBinary {
                    constant: index, ..
                }
                | Op::ConstantBinary {
                    constant: index, ..
                } => Some(constant(*index)?),
                _ => arg(1),
            };
            let (Some(left), Some(right)) = (left, right) else {
                return Ok(None);
            };
            if left == Rep::Bool || right == Rep::Bool {
                return Err(Unsupported);
            }
            let both_ints = left == Rep::Int && right == Rep::Int;
            match operator {
                // An integer and a float are compared as the general code
                // compares them.
                B::Equal if left != right => return Err(Unsupported),
                B::Equal | B::Greater | B::GreaterNum | B::Less | B::LessNum => Some(Rep::Bool),
                B::Modulo | B::ModuloNum | B::BitAnd | B::BitOr | B::BitXor if !both_ints => {
                    return Err(Unsupported);
                }
                _ if both_ints => Some(Rep::Int),
                _ => Some(Rep::Float),
            }
        }
    })
}

impl Lowering<'_> {
    /// At the end of the entry block's checks: enters the version the
    /// parameters' tags name, or else `general`, where the general code
    /// continues.
    pub(super) fn enter_versions(&mut self, versions: &[Version], general: CodeBlock) {
        // The block being filled ends here, as a block must before another
        // is filled: the versions are lowered, then the choice among them.
        let dispatch = self.builder.create_block();
        self.builder.ins().jump(dispatch, &[]);
        let entries = versions
            .iter()
            .map(|version| self.lower_version(version, general))
            .collect::<Vec<_>>();
        self.builder.switch_to_block(dispatch);
        let params = self.function.blocks[0].params.clone();
        for (version, entry) in versions.iter().zip(entries) {
            let mut matches = None;
            for (param, rep) in params.iter().zip(&version.params) {
                let slot = self.slot(param.index());
                let tag = self.load_tag(slot);
                let expected = if *rep == Rep::Int {
                    layout::INT
                } else {
                    layout::FLOAT
                };
                let is = self.builder.ins().icmp_imm(IntCC::Equal, tag, expected);
                matches = Some(match matches {
                    Some(previous) => self.builder.ins().band(previous, is),
                    None => is,
                });
            }
            let next = self.builder.create_block();
            let matches = matches.expect("a kernel has parameters");
            self.builder.ins().brif(matches, entry, &[], next, &[]);
            self.builder.switch_to_block(next);
        }
        self.builder.ins().jump(general, &[]);
    }

    /// Lowers `version`, which leaves to `general`; its entry, which loads
    /// the parameters from their slots.
    fn lower_version(&mut self, version: &Version, general: CodeBlock) -> CodeBlock {
        let function = self.function;
        let entry = self.builder.create_block();
        let blocks = function
            .blocks
            .iter()
            .map(|block| {
                let code = self.builder.create_block();
                for param in &block.params {
                    let kind = rep_type(version.reps[param.index()].expect("planned"));
                    self.builder.append_block_param(code, kind);
                }
                code
            })
            .collect::<Vec<_>>();
        let mut values: Vec<Option<CodeValue>> = vec![None; function.values.len()];

        self.builder.switch_to_block(entry);
        let mut arguments = Vec::new();
        for (param, rep) in function.blocks[0].params.iter().zip(&version.params) {
            let slot = self.slot(param.index());
            arguments.push(self.load(rep_type(*rep), slot, layout::PAYLOAD));
        }
        self.builder.ins().jump(blocks[0], &arguments);

        // Leaving: the iteration's parameters back to their slots, and the
        // iteration again in the general code.
        let leave = self.builder.create_block();
        let iteration = self.builder.block_params(blocks[0]).to_vec();
        self.builder.switch_to_block(leave);
        let params = function.blocks[0].params.clone();
        self.store_params(&params, &iteration, &version.params);
        self.builder.ins().jump(general, &[]);

        for (index, block) in function.blocks.iter().enumerate() {
            self.builder.switch_to_block(blocks[index]);
            for (param, value) in block
                .params
                .iter()
                .zip(self.builder.block_params(blocks[index]).to_vec())
            {
                values[param.index()] = Some(value);
            }
            for inst in &block.insts {
                let Some(result) = inst.results.first() else {
                    continue;
                };
                let value =
                    self.version_inst(function, &inst.op, &inst.args, version, &values, leave);
                values[result.index()] = Some(value);
            }
            self.version_terminator(&block.terminator, version, &blocks, &values, general);
        }
        entry
    }

    /// Lowers a block's `terminator` in `version`, whose blocks are
    /// `blocks` and values so far `values`, leaving to `general`.
    fn version_terminator(
        &mut self,
        terminator: &Terminator,
        version: &Version,
        blocks: &[CodeBlock],
        values: &[Option<CodeValue>],
        general: CodeBlock,
    ) {
        let value = |id: &ValueId| values[id.index()].expect("defined before use");
        let edge_args = |edge: &Edge| edge.args.iter().map(value).collect::<Vec<_>>();
        let params = self.function.blocks[0].params.clone();
        match terminator {
            Terminator::Jump(edge) => {
                let args = edge_args(edge);
                self.builder.ins().jump(blocks[edge.block.index()], &args);
            }
            Terminator::Branch {
                cond,
                then,
                otherwise,
            } => {
                let (then_args, otherwise_args) = (edge_args(then), edge_args(otherwise));
                let (then, otherwise) =
                    (blocks[then.block.index()], blocks[otherwise.block.index()]);
                if version.reps[cond.index()] == Some(Rep::Bool) {
                    self.builder.ins().brif(
                        value(cond),
                        then,
                        &then_args,
                        otherwise,
                        &otherwise_args,
                    );
                } else {
                    // A number is truthy.
                    self.builder.ins().jump(then, &then_args);
                }
            }
            Terminator::Return(result) => {
                let rep = version.reps[result.index()].expect("planned");
                let target = Place::At(self.builder.ins().load(
                    self.pointer,
                    MemFlags::trusted(),
                    self.context,
                    offsets::CONTEXT_RET,
                ));
                self.store_rep(target, value(result), rep);
                let status = self.builder.ins().iconst(types::I32, i64::from(RETURNED));
                self.builder.ins().jump(self.cleanup, &[status]);
            }
            Terminator::Recur(next) => {
                let next = next.iter().map(value).collect::<Vec<_>>();
                let attention = self.attention_requested();
                let attend = self.builder.create_block();
                self.builder
                    .ins()
                    .brif(attention, attend, &[], blocks[0], &next);
                // The next iteration runs in the general code, whose
                // `recur` attends to the thread.
                self.builder.switch_to_block(attend);
                self.store_params(&params, &next, &version.params);
                self.builder.ins().jump(general, &[]);
            }
            Terminator::Choose { .. } | Terminator::Stop => unreachable!("not a kernel"),
        }
    }

    /// Writes `values`, held as `reps`, to the slots of `params`.
    fn store_params(&mut self, params: &[ValueId], values: &[CodeValue], reps: &[Rep]) {
        for ((param, value), rep) in params.iter().zip(values).zip(reps) {
            let slot = self.slot(param.index());
            self.store_rep(slot, *value, *rep);
        }
    }

    /// Writes `value`, held as `rep`, to `place` as a `Value`.
    fn store_rep(&mut self, place: Place, value: CodeValue, rep: Rep) {
        match rep {
            Rep::Int => self.store_int(place, value),
            Rep::Float => self.store_float(place, value),
            Rep::Bool => self.store_bool(place, value),
        }
    }

    /// Computes one instruction in registers, leaving to `leave` where the
    /// general code would do something else.
    fn version_inst(
        &mut self,
        function: &Function,
        op: &Op,
        args: &[ValueId],
        version: &Version,
        values: &[Option<CodeValue>],
        leave: CodeBlock,
    ) -> CodeValue {
        use BinaryOperator as B;
        let rep = |id: ValueId| version.reps[id.index()].expect("planned");
        let constant =
            |builder: &mut cranelift_frontend::FunctionBuilder<'_>, index: usize| match function
                .constants
                .get(index)
            {
                Some(Constant::Value(Value::Int(value))) => {
                    (builder.ins().iconst(types::I64, *value), Rep::Int)
                }
                Some(Constant::Value(Value::Float(value))) => {
                    (builder.ins().f64const(*value), Rep::Float)
                }
                Some(Constant::Value(Value::Bool(value))) => (
                    builder.ins().iconst(types::I8, i64::from(*value)),
                    Rep::Bool,
                ),
                _ => unreachable!("planned"),
            };
        match op {
            Op::Constant(index) => constant(&mut self.builder, *index).0,
            Op::True | Op::False => self
                .builder
                .ins()
                .iconst(types::I8, i64::from(matches!(op, Op::True))),
            Op::Not => {
                let value = values[args[0].index()].expect("defined");
                self.builder.ins().icmp_imm(IntCC::Equal, value, 0)
            }
            Op::Negate => {
                let value = values[args[0].index()].expect("defined");
                if rep(args[0]) == Rep::Float {
                    return self.builder.ins().fneg(value);
                }
                let minimum = self.builder.ins().icmp_imm(IntCC::Equal, value, i64::MIN);
                self.leave_if(minimum, leave);
                self.builder.ins().ineg(value)
            }
            op => {
                let operator = binary_operator(op).expect("planned");
                let (mut a, left) = (values[args[0].index()].expect("defined"), rep(args[0]));
                let (mut b, right) = match op {
                    Op::LocalConstantBinary {
                        constant: index, ..
                    }
                    | Op::ConstantBinary {
                        constant: index, ..
                    } => constant(&mut self.builder, *index),
                    _ => (values[args[1].index()].expect("defined"), rep(args[1])),
                };
                if left == Rep::Int && right == Rep::Int {
                    return self.int_binary(operator, a, b, leave);
                }
                // As the VM's `numbers` converts an integer beside a float.
                if left == Rep::Int {
                    a = self.builder.ins().fcvt_from_sint(types::F64, a);
                }
                if right == Rep::Int {
                    b = self.builder.ins().fcvt_from_sint(types::F64, b);
                }
                let compare = |condition| Some(condition);
                let condition = match operator {
                    B::Equal => compare(FloatCC::Equal),
                    B::Greater | B::GreaterNum => compare(FloatCC::GreaterThan),
                    B::Less | B::LessNum => compare(FloatCC::LessThan),
                    _ => None,
                };
                if let Some(condition) = condition {
                    return self.builder.ins().fcmp(condition, a, b);
                }
                if matches!(operator, B::Divide | B::DivideNum) {
                    let zero = self.builder.ins().f64const(0.0);
                    let is_zero = self.builder.ins().fcmp(FloatCC::Equal, b, zero);
                    self.leave_if(is_zero, leave);
                }
                let value = match operator {
                    B::Add | B::AddNum => self.builder.ins().fadd(a, b),
                    B::Subtract | B::SubtractNum => self.builder.ins().fsub(a, b),
                    B::Multiply | B::MultiplyNum => self.builder.ins().fmul(a, b),
                    _ => self.builder.ins().fdiv(a, b),
                };
                // Finite exactly when `value - value` is zero.
                let difference = self.builder.ins().fsub(value, value);
                let zero = self.builder.ins().f64const(0.0);
                let not_finite = self.builder.ins().fcmp(FloatCC::NotEqual, difference, zero);
                self.leave_if(not_finite, leave);
                value
            }
        }
    }

    /// `a operator b` on two integers, leaving where the result would not be
    /// an integer or is an error.
    fn int_binary(
        &mut self,
        operator: BinaryOperator,
        a: CodeValue,
        b: CodeValue,
        leave: CodeBlock,
    ) -> CodeValue {
        use BinaryOperator as B;
        match operator {
            B::Add | B::AddNum | B::Subtract | B::SubtractNum | B::Multiply | B::MultiplyNum => {
                let (value, overflow) = match operator {
                    B::Add | B::AddNum => self.builder.ins().sadd_overflow(a, b),
                    B::Subtract | B::SubtractNum => self.builder.ins().ssub_overflow(a, b),
                    _ => self.builder.ins().smul_overflow(a, b),
                };
                self.leave_if(overflow, leave);
                value
            }
            B::Divide | B::DivideNum => {
                // A zero divisor and `i64::MIN / -1` are errors, and a
                // remainder makes a float: the general code's to give.
                let zero = self.builder.ins().icmp_imm(IntCC::Equal, b, 0);
                let minus_one = self.builder.ins().icmp_imm(IntCC::Equal, b, -1);
                let minimum = self.builder.ins().icmp_imm(IntCC::Equal, a, i64::MIN);
                let overflow = self.builder.ins().band(minus_one, minimum);
                let fault = self.builder.ins().bor(zero, overflow);
                self.leave_if(fault, leave);
                let remainder = self.builder.ins().srem(a, b);
                self.leave_if(remainder, leave);
                self.builder.ins().sdiv(a, b)
            }
            B::Modulo | B::ModuloNum => {
                let zero = self.builder.ins().icmp_imm(IntCC::Equal, b, 0);
                let minus_one = self.builder.ins().icmp_imm(IntCC::Equal, b, -1);
                let fault = self.builder.ins().bor(zero, minus_one);
                self.leave_if(fault, leave);
                self.builder.ins().srem(a, b)
            }
            B::BitAnd => self.builder.ins().band(a, b),
            B::BitOr => self.builder.ins().bor(a, b),
            B::BitXor => self.builder.ins().bxor(a, b),
            _ => {
                let condition = match operator {
                    B::Equal => IntCC::Equal,
                    B::Greater | B::GreaterNum => IntCC::SignedGreaterThan,
                    _ => IntCC::SignedLessThan,
                };
                self.builder.ins().icmp(condition, a, b)
            }
        }
    }

    /// Branches to `leave` when `condition` is nonzero, and continues in a
    /// new block otherwise.
    fn leave_if(&mut self, condition: CodeValue, leave: CodeBlock) {
        let next = self.builder.create_block();
        self.builder.ins().brif(condition, leave, &[], next, &[]);
        self.builder.switch_to_block(next);
    }
}

/// The machine type of a value held as `rep`.
fn rep_type(rep: Rep) -> types::Type {
    match rep {
        Rep::Int => types::I64,
        Rep::Float => types::F64,
        Rep::Bool => types::I8,
    }
}

#[cfg(test)]
mod tests {
    use super::{Rep, versions};
    use crate::{kinds::Kinds, lift::lift};

    /// The parameters' representations of each version of the function
    /// with `arity` parameters whose IR mentions `needle`.
    fn versions_of(source: &str, arity: usize, needle: &str) -> Vec<Vec<Rep>> {
        let program = slug_frontend::compile("test.slug", source).expect("test source compiles");
        let function = (0..program.chunk_count())
            .filter_map(|chunk| lift(&program, chunk).ok())
            .find(|function| function.arity == arity && function.to_string().contains(needle))
            .expect("the function lifts");
        let kinds = Kinds::of(&function);
        versions(&function, &kinds)
            .into_iter()
            .map(|version| version.params)
            .collect()
    }

    #[test]
    fn a_kernel_gets_a_version_for_each_combination_its_recur_keeps() {
        // `p + v` keeps `p` a float only when both are floats, and
        // `v + 0.25` keeps `v` one only when it is. A float `n` would
        // compare a float with the integer `0`, which the general code
        // does, so `n` is an integer.
        let versions = versions_of(
            "val advance = fn(n:num, p:num, v:num) { if (n == 0) { p } else { recur(n - 1, p + v, v + 0.25) } }\n\
             val main = fn() { println(advance(3, 0.0, 0.5)) }\n",
            3,
            "recur",
        );
        assert_eq!(versions, vec![vec![Rep::Int, Rep::Float, Rep::Float]]);
    }

    #[test]
    fn a_function_that_calls_or_builds_a_collection_is_not_a_kernel() {
        for (source, needle) in [
            (
                "val f = fn(n:num) { if (n == 0) { 0 } else { f(n - 1) } }\n\
                 val main = fn() { println(f(3)) }\n",
                "Call",
            ),
            (
                "val f = fn(n:num) { [n] }\nval main = fn() { println(f(3)) }\n",
                "List",
            ),
        ] {
            assert_eq!(
                versions_of(source, 1, needle),
                Vec::<Vec<Rep>>::new(),
                "{source}"
            );
        }
    }

    #[test]
    fn a_function_without_entry_checks_is_not_a_kernel() {
        let versions = versions_of(
            "val count = fn(n, acc) { if (n == 0) { acc } else { recur(n - 1, acc + 1) } }\n\
             val main = fn() { println(count(3, 0)) }\n",
            2,
            "recur",
        );
        assert!(versions.is_empty());
    }
}
