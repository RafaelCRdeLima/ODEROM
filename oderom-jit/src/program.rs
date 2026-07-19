//! The IR itself: a linear sequence of instructions in SSA form
//! (`ops[i]` only ever references indices `< i`, so evaluation is a
//! single forward pass -- no topological sort needed, the construction
//! order in [`crate::compile`] already is one) and [`Program::eval`], the
//! interpreter.

/// A constant, compared/hashed by bit pattern (exact equality, not
/// approximate) -- sufficient for common-subexpression elimination,
/// where the two constants being compared always come from literally
/// the same source `Scalar`, never from separately-computed floats that
/// merely round to the same value.
#[derive(Clone, Copy, Debug)]
pub struct ConstBits(f64);

impl PartialEq for ConstBits {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}
impl Eq for ConstBits {}
impl std::hash::Hash for ConstBits {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

/// One SSA instruction. Operands are indices into the enclosing
/// [`Program`]'s instruction list, always `<` the instruction's own
/// index.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Op {
    Const(ConstBits),
    /// An input variable, indexed into whatever slice `Program::eval`
    /// is called with (see [`crate::compile`] for how that indexing is
    /// chosen).
    Var(usize),
    Add(usize, usize),
    Mul(usize, usize),
    Pow(usize, i32),
    Sin(usize),
    Cos(usize),
}

impl Op {
    pub(crate) fn constant(value: f64) -> Op {
        Op::Const(ConstBits(value))
    }
}

/// A compiled expression: `ops`, plus which instruction is the result.
#[derive(Clone, Debug)]
pub struct Program {
    pub ops: Vec<Op>,
    pub output: usize,
}

impl Program {
    /// Runs the program with `inputs` as the values of its `Var(i)`
    /// slots (`inputs[i]`), returning the output instruction's value.
    pub fn eval(&self, inputs: &[f64]) -> f64 {
        let mut values = vec![0.0f64; self.ops.len()];
        for (i, op) in self.ops.iter().enumerate() {
            values[i] = match *op {
                Op::Const(c) => c.0,
                Op::Var(idx) => inputs[idx],
                Op::Add(a, b) => values[a] + values[b],
                Op::Mul(a, b) => values[a] * values[b],
                Op::Pow(a, n) => values[a].powi(n),
                Op::Sin(a) => values[a].sin(),
                Op::Cos(a) => values[a].cos(),
            };
        }
        values[self.output]
    }
}
