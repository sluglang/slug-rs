//! What kinds of value each IR value is known to hold, from the annotation
//! checks the compiler placed and the operations that follow them.
//!
//! An entry check that passed makes its parameter's kinds known: a function
//! annotated `fn(n:num)` begins with `CheckKinds` of `n`. The kinds then
//! follow what is computed: arithmetic on two numbers is a number, and a
//! comparison a boolean. Native code uses them to leave out checks they
//! make redundant, as a `recur` whose arguments are numbers needs not check
//! them again on re-entry.

use slug_vm::{BinaryOperator, Constant, Op, Value, check_kinds};

use crate::{
    compile::binary_operator,
    ir::{Function, ValueId},
};

/// The kinds of value each IR value is known to hold, as `check_kinds`
/// bits, by its index, and for each entry parameter the kinds its leading
/// entry check accepts.
pub(crate) struct Kinds {
    known: Vec<Option<u32>>,
    /// For each entry block parameter, the kinds the check the entry block
    /// begins with accepts, or `None` when none checks it.
    pub(crate) entry_checks: Vec<Option<u32>>,
    /// How many instructions the entry block's leading checks are.
    pub(crate) leading: usize,
}

impl Kinds {
    pub(crate) fn of(function: &Function) -> Self {
        let entry = &function.blocks[0];
        let mut entry_checks = vec![None; entry.params.len()];
        let mut leading = 0;
        for inst in &entry.insts {
            let (Op::CheckKinds { kinds, .. }, [checked]) = (&inst.op, inst.args.as_slice()) else {
                break;
            };
            let Some(index) = entry.params.iter().position(|param| param == checked) else {
                break;
            };
            if entry_checks[index].is_some() {
                break;
            }
            entry_checks[index] = Some(*kinds);
            leading += 1;
        }
        let mut known = vec![None; function.values.len()];
        for (param, kinds) in entry.params.iter().zip(&entry_checks) {
            known[param.index()] = *kinds;
        }
        // Loops are `recur`s, which restart the entry block, so every other
        // edge goes forward: a few passes settle the join parameters.
        loop {
            let mut changed = false;
            for block in &function.blocks {
                for inst in &block.insts {
                    let Some(result) = inst.results.first() else {
                        continue;
                    };
                    let kinds = result_kinds(function, &inst.op, &inst.args, &known);
                    if kinds.is_some() && known[result.index()] != kinds {
                        known[result.index()] = kinds;
                        changed = true;
                    }
                }
            }
            for (index, block) in function.blocks.iter().enumerate().skip(1) {
                for (position, param) in block.params.iter().enumerate() {
                    let incoming = function
                        .blocks
                        .iter()
                        .flat_map(|from| from.terminator.edges())
                        .filter(|edge| edge.block.index() == index)
                        .map(|edge| edge.args.get(position).and_then(|arg| known[arg.index()]))
                        .collect::<Vec<_>>();
                    let kinds = (!incoming.is_empty())
                        .then(|| {
                            incoming
                                .iter()
                                .try_fold(0, |all, kinds| Some(all | (*kinds)?))
                        })
                        .flatten();
                    if kinds.is_some() && known[param.index()] != kinds {
                        known[param.index()] = kinds;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        Self {
            known,
            entry_checks,
            leading,
        }
    }

    /// Whether `value` is known to be of one of `kinds`.
    pub(crate) fn passes(&self, value: ValueId, kinds: u32) -> bool {
        self.known[value.index()].is_some_and(|known| known & !kinds == 0)
    }

    /// Whether a `recur` with `arguments` passes the entry block's leading
    /// checks, so it may restart past them.
    pub(crate) fn recur_passes(&self, arguments: &[ValueId]) -> bool {
        self.leading > 0
            && self
                .entry_checks
                .iter()
                .zip(arguments)
                .all(|(check, argument)| check.is_none_or(|kinds| self.passes(*argument, kinds)))
    }
}

/// The kinds an instruction's result is known to hold, from its operands'.
fn result_kinds(
    function: &Function,
    op: &Op,
    args: &[ValueId],
    known: &[Option<u32>],
) -> Option<u32> {
    use BinaryOperator as B;
    let constant = |index: usize| match function.constants.get(index) {
        Some(Constant::Value(value)) => kind_of(value),
        _ => None,
    };
    let arg = |index: usize| args.get(index).and_then(|arg| known[arg.index()]);
    match op {
        Op::Constant(index) => constant(*index),
        Op::Nil => Some(check_kinds::NIL),
        Op::True | Op::False | Op::Not => Some(check_kinds::BOOL),
        Op::Negate => arg(0).filter(|kinds| *kinds == check_kinds::NUM),
        op => {
            let operator = binary_operator(op)?;
            if matches!(
                operator,
                B::Equal | B::Greater | B::GreaterNum | B::Less | B::LessNum
            ) {
                return Some(check_kinds::BOOL);
            }
            let right = match op {
                Op::LocalConstantBinary {
                    constant: index, ..
                }
                | Op::ConstantBinary {
                    constant: index, ..
                } => constant(*index),
                _ => arg(1),
            };
            (arg(0) == Some(check_kinds::NUM) && right == Some(check_kinds::NUM))
                .then_some(check_kinds::NUM)
        }
    }
}

/// The kind `value` is, as one `check_kinds` bit.
fn kind_of(value: &Value) -> Option<u32> {
    (0..14)
        .map(|bit| 1 << bit)
        .find(|kind| value.has_kind_in(*kind))
}

#[cfg(test)]
mod tests {
    use super::Kinds;
    use crate::{ir::Terminator, lift::lift};
    use slug_vm::check_kinds;

    /// The kinds of the function with `arity` parameters whose IR mentions
    /// `needle`, with whether its `recur` passes its entry checks.
    fn kinds_of(source: &str, arity: usize, needle: &str) -> (Kinds, Option<bool>) {
        let program = slug_frontend::compile("test.slug", source).expect("test source compiles");
        let function = (0..program.chunk_count())
            .filter_map(|chunk| lift(&program, chunk).ok())
            .find(|function| function.arity == arity && function.to_string().contains(needle))
            .expect("the function lifts");
        let kinds = Kinds::of(&function);
        let recur = function
            .blocks
            .iter()
            .find_map(|block| match &block.terminator {
                Terminator::Recur(arguments) => Some(kinds.recur_passes(arguments)),
                _ => None,
            });
        (kinds, recur)
    }

    #[test]
    fn a_recur_of_numbers_computed_from_checked_numbers_passes_the_entry_checks() {
        let (kinds, recur) = kinds_of(
            "val advance = fn(n:num, x:num) { if (n == 0) { x } else { recur(n - 1, x + 0.5) } }\n\
             val main = fn() { println(advance(3, 0.0)) }\n",
            2,
            "recur",
        );
        assert_eq!(kinds.leading, 2);
        assert_eq!(kinds.entry_checks, vec![Some(check_kinds::NUM); 2]);
        assert_eq!(recur, Some(true));
    }

    #[test]
    fn a_recur_of_a_value_not_known_checks_again() {
        let (_, recur) = kinds_of(
            "val dynamic = fn(v) { v }\n\
             val count = fn(n:num, acc:num) { if (n == 0) { acc } else { recur(n - 1, dynamic(acc)) } }\n\
             val main = fn() { println(count(3, 0)) }\n",
            2,
            "recur",
        );
        assert_eq!(recur, Some(false));
    }

    #[test]
    fn a_function_with_no_entry_checks_has_none_to_skip() {
        let (kinds, recur) = kinds_of(
            "val count = fn(n, acc) { if (n == 0) { acc } else { recur(n - 1, acc + 1) } }\n\
             val main = fn() { println(count(3, 0)) }\n",
            2,
            "recur",
        );
        assert_eq!(kinds.leading, 0);
        assert_eq!(recur, Some(false));
    }
}
