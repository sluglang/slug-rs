//! Numeric kernels with their numbers in registers.
//!
//! A function whose entry checks prove each parameter a number, and whose
//! body only computes with numbers and booleans, calls functions on
//! numbers, branches, and returns or recurs, gets a version for each
//! combination of its parameters being integers or floats that its
//! `recur`s keep. A version keeps every value in a register, with no tag,
//! and writes `Value`s only where it leaves its registers: for its result,
//! a call's arguments, and an exit. The entry, past the checks, takes the
//! version the arguments' tags name.
//!
//! Where a version cannot compute an instruction as the general code would
//! (an overflow, a division with a remainder, which Slug makes a float, a
//! result that is not finite), at a `recur` when the thread needs
//! attention, and where a call returns a value held otherwise than planned,
//! it writes the values the instruction's snapshot names to their slots
//! and leaves to the interpreter there, as the general code does. A call
//! writes them before it is made, and is lowered as the general code's, so
//! a callee that declines or exits leaves as it does there.

use cranelift_codegen::ir::{
    Block as CodeBlock, InstBuilder, MemFlags, Value as CodeValue,
    condcodes::{FloatCC, IntCC},
    types,
};
use slug_vm::{BinaryOperator, Constant, Op, Value, check_kinds};

use super::{Callee, Lowering, Place, binary_operator};
use crate::{
    ir::{Edge, Function, Inst, Local, Snapshot, Terminator, ValueDef, ValueId},
    kinds::Kinds,
    runtime::{RETURNED, layout},
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
pub(super) const MAX_VERSIONS: usize = 8;

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
/// number, and nothing but arithmetic, comparisons, constants, calls whose
/// results are computed with, branches, returns, and `recur`s.
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
                    Op::CallGlobalPooled { count, .. } => {
                        *count >= 1 && computed_with(function, inst.results[0])
                    }
                    op => binary_operator(op).is_some_and(supported),
                })
    })
}

/// Whether every use of `value`, of which there is one, computes with it
/// as a number, or passes it on: so a call made for what it does, whose
/// result is not a number, as `println`'s, keeps its function general.
fn computed_with(function: &Function, value: ValueId) -> bool {
    let mut uses = 0;
    for block in &function.blocks {
        for inst in &block.insts {
            if !inst.args.contains(&value) {
                continue;
            }
            uses += 1;
            let numeric = matches!(inst.op, Op::Negate | Op::CallGlobalPooled { .. })
                || binary_operator(&inst.op).is_some_and(supported);
            if !numeric {
                return false;
            }
        }
        match &block.terminator {
            Terminator::Branch { cond, .. } if *cond == value => return false,
            Terminator::Return(returned) if *returned == value => uses += 1,
            Terminator::Recur(arguments) if arguments.contains(&value) => uses += 1,
            terminator => {
                uses += terminator
                    .edges()
                    .iter()
                    .filter(|edge| edge.args.contains(&value))
                    .count();
            }
        }
    }
    uses > 0
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
        // A call's result is taken to be held as its first argument is,
        // as a recursive call's is; a result held otherwise leaves.
        Op::CallGlobalPooled { .. } => {
            let reps = (0..args.len()).map(arg).collect::<Option<Vec<_>>>();
            match reps {
                None => None,
                Some(reps) if reps.contains(&Rep::Bool) => return Err(Unsupported),
                Some(reps) => reps.first().copied(),
            }
        }
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
            .map(|version| self.lower_version(version))
            .collect::<Vec<_>>();
        self.builder.switch_to_block(dispatch);
        let params = self.function.blocks[0].params.clone();
        for (version, entry) in versions.iter().zip(entries) {
            let mut matches = None;
            for (param, rep) in params.iter().zip(&version.params) {
                let slot = self.slot(param.index());
                let tag = self.load_tag(slot);
                let is = self
                    .builder
                    .ins()
                    .icmp_imm(IntCC::Equal, tag, rep_tag(*rep));
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

    /// Lowers `version`; its entry, which loads the parameters from their
    /// slots.
    fn lower_version(&mut self, version: &Version) -> CodeBlock {
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

        for (index, block) in function.blocks.iter().enumerate() {
            self.builder.switch_to_block(blocks[index]);
            for (param, value) in block
                .params
                .iter()
                .zip(self.builder.block_params(blocks[index]).to_vec())
            {
                values[param.index()] = Some(value);
            }
            for (position, inst) in block.insts.iter().enumerate() {
                let Some(result) = inst.results.first() else {
                    continue;
                };
                let value = self.version_inst(inst, (index, position), version, &values);
                values[result.index()] = Some(value);
            }
            self.version_terminator(block, version, &blocks, &values);
        }
        entry
    }

    /// Lowers `block`'s terminator in `version`, whose blocks are `blocks`
    /// and values so far `values`.
    fn version_terminator(
        &mut self,
        block: &crate::ir::Block,
        version: &Version,
        blocks: &[CodeBlock],
        values: &[Option<CodeValue>],
    ) {
        let value = |id: &ValueId| values[id.index()].expect("defined before use");
        let edge_args = |edge: &Edge| edge.args.iter().map(value).collect::<Vec<_>>();
        match &block.terminator {
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
                    super::offsets::CONTEXT_RET,
                ));
                self.store_rep(target, value(result), rep);
                let status = self.builder.ins().iconst(types::I32, i64::from(RETURNED));
                self.builder.ins().jump(self.cleanup, &[status]);
            }
            Terminator::Recur(next) => {
                let next = next.iter().map(value).collect::<Vec<_>>();
                let inst = block.insts.last().expect("a recur block has its recur");
                let attention = self.attention_requested();
                let attend = self.builder.create_block();
                self.builder
                    .ins()
                    .brif(attention, attend, &[], blocks[0], &next);
                // The interpreter attends to the thread, then recurs.
                self.builder.switch_to_block(attend);
                self.leave_at(inst, version, values);
            }
            Terminator::Choose { .. } | Terminator::Stop => unreachable!("not a kernel"),
        }
    }

    /// Writes the values `snapshot` names, from their registers, to their
    /// slots, as the general code would hold them there.
    fn materialize(
        &mut self,
        snapshot: &Snapshot,
        version: &Version,
        values: &[Option<CodeValue>],
    ) {
        let function = self.function;
        let locals = snapshot.locals.iter().filter_map(|local| match local {
            Local::Value(value) => Some(*value),
            Local::Cell => None,
        });
        let stack = snapshot
            .stack
            .iter()
            .copied()
            .filter(|value| !matches!(function.def(*value), ValueDef::Below(_)));
        for value in locals.chain(stack).collect::<Vec<_>>() {
            if let (Some(register), Some(rep)) =
                (values[value.index()], version.reps[value.index()])
            {
                let slot = self.slot(value.index());
                self.store_rep(slot, register, rep);
            }
        }
    }

    /// Leaves to the interpreter before `inst`, with the frame its snapshot
    /// names.
    fn leave_at(&mut self, inst: &Inst, version: &Version, values: &[Option<CodeValue>]) {
        self.materialize(&inst.snapshot, version, values);
        let exit = self.exit_block(inst);
        self.builder.ins().jump(exit, &[]);
    }

    /// Branches to leave before `inst` when `condition` is nonzero, and
    /// continues in a new block otherwise.
    fn leave_if(
        &mut self,
        condition: CodeValue,
        inst: &Inst,
        version: &Version,
        values: &[Option<CodeValue>],
    ) {
        let leave = self.builder.create_block();
        let next = self.builder.create_block();
        self.builder.ins().brif(condition, leave, &[], next, &[]);
        self.builder.switch_to_block(leave);
        self.leave_at(inst, version, values);
        self.builder.switch_to_block(next);
    }

    /// Writes `value`, held as `rep`, to `place` as a `Value`.
    fn store_rep(&mut self, place: Place, value: CodeValue, rep: Rep) {
        match rep {
            Rep::Int => self.store_int(place, value),
            Rep::Float => self.store_float(place, value),
            Rep::Bool => self.store_bool(place, value),
        }
    }

    /// Computes `inst`, instruction `position` of its block, in registers.
    fn version_inst(
        &mut self,
        inst: &Inst,
        position: (usize, usize),
        version: &Version,
        values: &[Option<CodeValue>],
    ) -> CodeValue {
        let function = self.function;
        let rep = |id: ValueId| version.reps[id.index()].expect("planned");
        let register = |id: ValueId| values[id.index()].expect("defined before use");
        let (op, args) = (&inst.op, &inst.args);
        match op {
            Op::Constant(index) => self.version_constant(function, *index).0,
            Op::True | Op::False => self
                .builder
                .ins()
                .iconst(types::I8, i64::from(matches!(op, Op::True))),
            Op::Not => {
                let value = register(args[0]);
                self.builder.ins().icmp_imm(IntCC::Equal, value, 0)
            }
            Op::Negate => {
                let value = register(args[0]);
                if rep(args[0]) == Rep::Float {
                    return self.builder.ins().fneg(value);
                }
                let minimum = self.builder.ins().icmp_imm(IntCC::Equal, value, i64::MIN);
                self.leave_if(minimum, inst, version, values);
                self.builder.ins().ineg(value)
            }
            Op::CallGlobalPooled { name, count, .. } => {
                self.version_call(inst, (name.get(), *count), position, version, values)
            }
            op => {
                let operator = binary_operator(op).expect("planned");
                let (a, left) = (register(args[0]), rep(args[0]));
                let (b, right) = match op {
                    Op::LocalConstantBinary { constant, .. }
                    | Op::ConstantBinary { constant, .. } => {
                        self.version_constant(function, *constant)
                    }
                    _ => (register(args[1]), rep(args[1])),
                };
                if left == Rep::Int && right == Rep::Int {
                    return self.int_binary(operator, a, b, inst, version, values);
                }
                self.float_binary(operator, (a, left), (b, right), inst, version, values)
            }
        }
    }

    /// Constant `index` of `function`, as a register, and how it is held.
    fn version_constant(&mut self, function: &Function, index: usize) -> (CodeValue, Rep) {
        match function.constants.get(index) {
            Some(Constant::Value(Value::Int(value))) => {
                (self.builder.ins().iconst(types::I64, *value), Rep::Int)
            }
            Some(Constant::Value(Value::Float(value))) => {
                (self.builder.ins().f64const(*value), Rep::Float)
            }
            Some(Constant::Value(Value::Bool(value))) => (
                self.builder.ins().iconst(types::I8, i64::from(*value)),
                Rep::Bool,
            ),
            _ => unreachable!("planned"),
        }
    }

    /// A call of module-level name `name` with `count` arguments: the frame
    /// written to its slots, the general code's call, and its result taken
    /// into a register, leaving after the call when it is held otherwise.
    fn version_call(
        &mut self,
        inst: &Inst,
        (name, count): (u32, usize),
        position: (usize, usize),
        version: &Version,
        values: &[Option<CodeValue>],
    ) -> CodeValue {
        self.materialize(&inst.snapshot, version, values);
        self.lower_call(inst, Callee::Global(name), (count, false), position)
            .expect("a kernel's call lowers");
        let result = inst.results[0];
        let rep = version.reps[result.index()].expect("planned");
        let slot = self.slot(result.index());
        let tag = self.load_tag(slot);
        let other = self
            .builder
            .ins()
            .icmp_imm(IntCC::NotEqual, tag, rep_tag(rep));
        let leave = self.builder.create_block();
        let next = self.builder.create_block();
        self.builder.ins().brif(other, leave, &[], next, &[]);
        // The interpreter continues past the call, with its result where the
        // callee wrote it.
        self.builder.switch_to_block(leave);
        let mut snapshot = inst.snapshot.clone();
        let below = snapshot.stack.len() - inst.args.len();
        snapshot.stack.truncate(below);
        snapshot.stack.push(result);
        self.materialize(&snapshot, version, values);
        let exit = self.exit_with(inst.ip + 1, &snapshot, 0, super::Leaving::Exit);
        self.builder.ins().jump(exit, &[]);
        self.builder.switch_to_block(next);
        self.load(rep_type(rep), slot, layout::PAYLOAD)
    }

    /// `a operator b` on two numbers, of which one is a float, converting the
    /// other as the VM's `numbers` converts it.
    fn float_binary(
        &mut self,
        operator: BinaryOperator,
        (mut a, left): (CodeValue, Rep),
        (mut b, right): (CodeValue, Rep),
        inst: &Inst,
        version: &Version,
        values: &[Option<CodeValue>],
    ) -> CodeValue {
        use BinaryOperator as B;
        if left == Rep::Int {
            a = self.builder.ins().fcvt_from_sint(types::F64, a);
        }
        if right == Rep::Int {
            b = self.builder.ins().fcvt_from_sint(types::F64, b);
        }
        let condition = match operator {
            B::Equal => Some(FloatCC::Equal),
            B::Greater | B::GreaterNum => Some(FloatCC::GreaterThan),
            B::Less | B::LessNum => Some(FloatCC::LessThan),
            _ => None,
        };
        if let Some(condition) = condition {
            return self.builder.ins().fcmp(condition, a, b);
        }
        if matches!(operator, B::Divide | B::DivideNum) {
            let zero = self.builder.ins().f64const(0.0);
            let is_zero = self.builder.ins().fcmp(FloatCC::Equal, b, zero);
            self.leave_if(is_zero, inst, version, values);
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
        self.leave_if(not_finite, inst, version, values);
        value
    }

    /// `a operator b` on two integers, leaving where the result would not be
    /// an integer or is an error.
    fn int_binary(
        &mut self,
        operator: BinaryOperator,
        a: CodeValue,
        b: CodeValue,
        inst: &Inst,
        version: &Version,
        values: &[Option<CodeValue>],
    ) -> CodeValue {
        use BinaryOperator as B;
        match operator {
            B::Add | B::AddNum | B::Subtract | B::SubtractNum | B::Multiply | B::MultiplyNum => {
                let (value, overflow) = match operator {
                    B::Add | B::AddNum => self.builder.ins().sadd_overflow(a, b),
                    B::Subtract | B::SubtractNum => self.builder.ins().ssub_overflow(a, b),
                    _ => self.builder.ins().smul_overflow(a, b),
                };
                self.leave_if(overflow, inst, version, values);
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
                self.leave_if(fault, inst, version, values);
                let remainder = self.builder.ins().srem(a, b);
                self.leave_if(remainder, inst, version, values);
                self.builder.ins().sdiv(a, b)
            }
            B::Modulo | B::ModuloNum => {
                let zero = self.builder.ins().icmp_imm(IntCC::Equal, b, 0);
                let minus_one = self.builder.ins().icmp_imm(IntCC::Equal, b, -1);
                let fault = self.builder.ins().bor(zero, minus_one);
                self.leave_if(fault, inst, version, values);
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
}

/// The tag of a `Value` holding a number held as `rep`.
fn rep_tag(rep: Rep) -> i64 {
    match rep {
        Rep::Int => layout::INT,
        Rep::Float => layout::FLOAT,
        Rep::Bool => layout::BOOL,
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
    fn a_call_computed_with_is_part_of_a_kernel() {
        // The recursive call's result is taken to be an integer, as `n` is;
        // a float `n` compares a float with `0`, which leaves it general.
        let versions = versions_of(
            "val f = fn(n:num) { if (n == 0) { 0 } else { f(n - 1) + 1 } }\n\
             val main = fn() { println(f(3)) }\n",
            1,
            "Call",
        );
        assert_eq!(versions, vec![vec![Rep::Int]]);
    }

    #[test]
    fn a_call_made_for_what_it_does_or_a_collection_is_not_a_kernel() {
        for (source, needle) in [
            (
                "val f = fn(n:num) { println(n)\n n + 1 }\n\
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
