use string_interner::{symbol::SymbolU32, DefaultBackend, DefaultHashBuilder};

use crate::Position;

/// OpCode is a number that specifies the type of the instruction.
///
/// # Notes
///
/// If it was for performances purposes, the following options could be considered:
/// + Having the opcode designed to be as close as possbible to existing lower-level instructions
/// + Having specialized opcode for constant
///
/// We don't have a `OpCode::NotEqual` because we will transform `a != b` to `!(a == b)` to demonstrated
/// that bytecode can deviate from the actual user's code as long as they behave similarly. This is
/// also applied for operator `<=` and operator `>=`.
///
/// `a <= b` does not equals equivalent to `!(a > b)`, similarly with greater and greater or equal.
/// According to [IEEE 754] all comparison operators return `false` when an operand is `NaN`. These
/// are implementation details that we should keep in mind when making a real language.
///
/// [IEEE 754]: https://en.wikipedia.org/wiki/IEEE_754
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum OpCode {
    /// Pop the top of the stack
    Pop,
    /// Print an expression in human readable format
    Print,
    /// Return from the current function
    Return,
    /// Set the value of a global variable
    GetLocal(u8),
    /// Set the value of a local variable
    SetLocal(u8),
    /// Pop the top of the stack and define a variable initialized with that value.
    DefineGlobal(u8),
    /// Get the value of a global variable
    GetGlobal(u8),
    /// Set the value of a global variable
    SetGlobal(u8),
    /// Load a constant
    Constant(u8),
    /// Load a `nil` value
    Nil,
    /// Load a `true` value
    True,
    /// Load a `false` value
    False,
    /// Apply logical `not` to a single boolean operand
    Not,
    /// Negate a single number operand
    Negate,
    /// Check for equality between 2 operands.
    Equal,
    /// Compare if the first operand is greater than the second
    Greater,
    /// Compare if the first operand is less than the second
    Less,
    /// Add two number operands or two string operands
    Add,
    /// Subtract two number operands
    Subtract,
    /// Multiply two number operands
    Multiply,
    /// Divide two number operands
    Divide,
}

/// Default string interner
pub type StringInterner<B = DefaultBackend<StringId>, H = DefaultHashBuilder> =
    string_interner::StringInterner<StringId, B, H>;

/// Interned string id
pub type StringId = SymbolU32;

/// This represents a Lox type and its data at.
#[derive(Debug, Clone)]
pub enum Value {
    /// A nothing value in Lox
    Nil,
    /// A boolean value in Lox
    Bool(bool),
    /// A number value in Lox
    Number(f64),
    /// A heap allocated string
    ///
    /// # Notes
    ///
    /// To improve memory usage, we should separated string into 2 types, one that owns its
    /// character array and one that is "constant" such that it points to the original source
    /// or some non-freeable location.
    String(StringId),
}

impl Value {
    /// Return true if the value is `nil` or `false`. Otherwise, return false.
    pub fn is_falsey(&self) -> bool {
        match self {
            Self::Bool(b) => !b,
            Self::Nil => true,
            _ => false,
        }
    }

    /// Get the string representation as this value.
    pub fn as_string(&self, strings: &StringInterner) -> String {
        match self {
            Self::Nil => "nil".to_string(),
            Self::Bool(b) => format!("{}", b),
            Self::Number(n) => format!("{}", n),
            Value::String(id) => strings
                .resolve(*id)
                .expect("String must be allocated before access.")
                .to_string(),
        }
    }

    /// Check for equality between two values of the same type. If the operands are of different
    /// types, return `false`.
    pub fn equal(&self, other: &Value) -> bool {
        match (self, other) {
            (Self::Nil, Self::Nil) => true,
            (Self::Bool(v1), Self::Bool(v2)) => v1 == v2,
            (Self::Number(v1), Self::Number(v2)) => (v1 - v2).abs() < f64::EPSILON,
            (Self::String(s1), Self::String(s2)) => s1 == s2,
            _ => false,
        }
    }
}

/// A chunk holds a sequence of instructions to be executes and their data
///
/// ```
/// use rlox::{Chunk, OpCode, Position, Value};
///
/// let mut chunk = Chunk::default();
/// let const_id = chunk.write_const(Value::Number(1.0));
/// assert!(matches!(chunk.read_const(const_id), &Value::Number(1.0)));
///
/// chunk.write_instruction(OpCode::Constant(const_id), Position::default());
/// assert!(matches!(
///     chunk.read_instruction(0),
///     (&OpCode::Constant(cost_id), &Position { line: 1, column : 1 }),
/// ));
/// ```
#[derive(Default, Debug)]
pub struct Chunk {
    instructions: Vec<OpCode>,
    constants: Vec<Value>,
    positions: Vec<Position>,
}

impl Chunk {
    /// Add a new instruction to the chunk.
    pub fn write_instruction(&mut self, code: OpCode, pos: Position) {
        self.instructions.push(code);
        self.positions.push(pos);
    }

    /// Read the instruction at the index.
    pub fn read_instruction(&self, idx: usize) -> (&OpCode, &Position) {
        (&self.instructions[idx], &self.positions[idx])
    }

    /// Add a constant value to the chunk and return it position in the Vec
    pub fn write_const(&mut self, val: Value) -> u8 {
        self.constants.push(val);
        self.constants.len() as u8 - 1
    }

    /// Read the constant at the given index
    pub fn read_const(&self, idx: u8) -> &Value {
        &self.constants[idx as usize]
    }
}

/// Go through the instructions in the chunk and display them in human-readable format.
#[cfg(debug_assertions)]
pub fn disassemble_chunk(chunk: &Chunk, name: &str, strings: &StringInterner) {
    println!("== {} ==", name);
    for i in 0..chunk.instructions.len() {
        disassemble_instruction(chunk, i, strings);
    }
}

/// Display an instruction in human readable format.
#[cfg(debug_assertions)]
pub fn disassemble_instruction(chunk: &Chunk, idx: usize, strings: &StringInterner) {
    print!("{:04} ", idx);
    if idx > 0 && chunk.positions[idx].line == chunk.positions[idx - 1].line {
        print!("   | ");
    } else {
        print!("{:4} ", chunk.positions[idx].line);
    }

    let constant_instruction = |op_repr: &str, const_id: u8| match chunk.read_const(const_id) {
        Value::String(id) => println!(
            "{:-16} {:4} {}",
            op_repr,
            const_id,
            strings
                .resolve(*id)
                .expect("String must be allocated before access.")
        ),
        val => println!("{:-16} {:4} {}", op_repr, const_id, val.as_string(strings)),
    };

    let byte_instruction = |op_repr: &str, slot: u8| println!("{:-16} {:4}", op_repr, slot);

    match chunk.instructions[idx] {
        OpCode::Pop => println!("OP_POP"),
        OpCode::Print => println!("OP_PRINT"),
        OpCode::Return => println!("OP_RETURN"),
        OpCode::GetLocal(ref slot) => byte_instruction("OP_GET_LOCAL", *slot),
        OpCode::SetLocal(ref slot) => byte_instruction("OP_SET_LOCAL", *slot),
        OpCode::DefineGlobal(ref const_id) => constant_instruction("OP_DEFINE_GLOBAL", *const_id),
        OpCode::GetGlobal(ref const_id) => constant_instruction("OP_GET_GLOBAL", *const_id),
        OpCode::SetGlobal(ref const_id) => constant_instruction("OP_SET_GLOBAL", *const_id),
        OpCode::Constant(ref const_id) => constant_instruction("OP_CONSTANT", *const_id),
        OpCode::Nil => println!("OP_NIL"),
        OpCode::True => println!("OP_TRUE"),
        OpCode::False => println!("OP_FALSE"),
        OpCode::Not => println!("OP_NOT"),
        OpCode::Negate => println!("OP_NEGATE"),
        OpCode::Equal => println!("OP_EQUAL"),
        OpCode::Greater => println!("OP_GREATER"),
        OpCode::Less => println!("OP_LESS"),
        OpCode::Add => println!("OP_ADD"),
        OpCode::Subtract => println!("OP_SUBTRACT"),
        OpCode::Multiply => println!("OP_MULTIPLY"),
        OpCode::Divide => println!("OP_DIVIDE"),
    }
}
