use std::rc::Rc;

use crate::{
    intern, token, Chunk, ObjFun, OpCode, Position, Scanner, StringId, Token, Value, MAX_STACK,
};

#[cfg(debug_assertions)]
use crate::disassemble_chunk;

/// Maximum number of parameters a function can take
pub const MAX_PARAMS: usize = 255;

/// Maximum number of parameters a function can take
pub const MAX_LOCAL_VARIABLES: usize = 256;

/// Maximum number of parameters a function can take
pub const MAX_CHUNK_CONSTANTS: usize = 256;

/// Function object's type.
///
/// This is used to so that the compiler knows what kind of chunk it's current compilling.
/// We are treating the entire script as a implicit function.
#[derive(Debug, PartialEq, Eq)]
pub enum FunType {
    /// The compiled chunk is of a function
    Function,
    /// The compiled chunk is of the input script
    Script,
}

/// Scan for tokens and emit corresponding bytecodes.
///
/// # The Lox Compiler
///
/// Lox uses lexical scoping so the compiler knows where it is within the stack while parsing the
/// source code. We are simulating the virtual machine's stack so at runtime we can pre-allocate
/// the needed space on to the stack, and access locals through array index for better preformance.
///
/// ## Locals Stack
///
/// ```
/// {
///     var a = 1;             // STACK: [ 1 ]
///     {
///         var b = 2;         // STACK: [ 1 ] [ 2 ]
///         {
///             var c = 3;     // STACK: [ 1 ] [ 2 ] [ 3 ]
///             {
///                 var d = 4; // STACK: [ 1 ] [ 2 ] [ 3 ] [ 4 ]
///             }              // STACK: [ 1 ] [ 2 ] [ 3 ] [ x ]
///
///             var e = 5;     // STACK: [ 1 ] [ 2 ] [ 3 ] [ 5 ]
///         }                  // STACK: [ 1 ] [ 2 ] [ x ] [ x ]
///     }                      // STACK: [ 1 ] [ x ]
///
///     var f = 6;             // STACK: [ 1 ] [ 6 ]
///     {
///         var g = 7;         // STACK: [ 1 ] [ 6 ] [ 7 ]
///     }                      // STACK: [ 1 ] [ 6 ] [ x ]
/// }                          // STACK: [ x ] [ x ]
/// ```
///
/// # Grammars
///
/// ```text
/// program    --> decl* EOF ;
/// decl       --> classDecl
///              | funDecl
///              | varDecl
///              | stmt ;
/// classDecl  --> "class" IDENT ( "<" IDENT )? "{" function* "}" ;
/// funDecl    --> "fun" function ;
/// function   --> IDENT "(" params? ")" block ;
/// params     --> IDENT ( "," IDENT )* ;
/// varDecl    --> "var" IDENT ( "=" expr )? ";" ;
/// stmt       --> block
///              | exprStmt
///              | forStmt
///              | ifStmt
///              | printStmt
///              | returnStmt
///              | whileStmt ;
/// block      --> "{" decl* "}" ;
/// exprStmt   --> expr ";" ;
/// forStmt    --> "for" "(" ( varDecl | exprStmt | ";" ) expr? ";" expr? ")" stmt ;
/// ifStmt     --> "if" "(" expr ")" stmt ( "else" stmt )? ;
/// printStmt  --> "print" expr ";" ;
/// returnStmt --> "return" expr? ";" ;
/// whileStmt  --> "while" "(" expr ")" stmt ;
/// expr       --> assign ;
/// assign     --> ( call "." )? IDENT "=" expr ";"
///              | or ;
/// or         --> and ( "or" and )* ;
/// and        --> equality ( "and" equality )* ;
/// equality   --> comparison ( ( "!=" | "==" ) comparison )* ;
/// comparison --> term ( ( ">" | ">=" | "<" | "<=" ) term )* ;
/// term       --> factor ( ( "-" | "+" ) factor )* ;
/// factor     --> unary ( ( "/" | "*" ) unary )* ;
/// unary      --> ( "!" | "-" ) unary
///              | call ;
/// call       --> primary ( "(" args? ")" | "." IDENT )* ;
/// args       --> expr ( "," expr )* ;
/// primary    --> IDENT | NUMBER | STRING
///              | "this" | "super" "." IDENT
///              | "true" | "false" | "nil"
///              | "(" expr ")" ;
///
#[derive(Debug)]
pub struct Compiler<'a> {
    scanner: Scanner<'a>,
    current_token: Token<'a>,
    previous_token: Token<'a>,
    had_error: bool,
    panic: bool,
    // Avoid having a linked list of compiler, solution found from
    // https://github.com/tdp2110/crafting-interpreters-rs/blob/trunk/src/compiler.rs
    nestings: Vec<Nesting>,
}

impl<'a> Compiler<'a> {
    /// Create a new parser
    pub fn new(src: &'a str) -> Self {
        Self {
            scanner: Scanner::new(src),
            current_token: Token {
                typ: token::Type::Eof,
                lexeme: "",
                pos: Position::default(),
            },
            previous_token: Token {
                typ: token::Type::Eof,
                lexeme: "",
                pos: Position::default(),
            },
            had_error: false,
            panic: false,
            nestings: vec![Nesting::new(ObjFun::default(), FunType::Script)],
        }
    }

    /// Starts building the bytecode chunk
    pub fn compile(&mut self) {
        self.advance();
        while !self.check(token::Type::Eof) {
            self.declaration();
        }
    }

    /// Return the compiled bytecode chunk if the process finishes without error
    pub fn finish(&mut self) -> Option<ObjFun> {
        if self.had_error {
            return None;
        }
        self.emit_return();
        #[cfg(debug_assertions)]
        disassemble_chunk(
            &self.nest().fun.chunk,
            format!("{}", self.nest().fun).as_str(),
        );
        Some(self.nestings.pop().expect("Cannot be empty").fun)
    }

    fn chunk(&mut self) -> &mut Chunk {
        &mut self.nest_mut().fun.chunk
    }

    fn nest(&self) -> &Nesting {
        self.nestings.last().expect("Cannot be empty")
    }

    fn nest_mut(&mut self) -> &mut Nesting {
        self.nestings.last_mut().expect("Cannot be empty")
    }

    fn make_const(&mut self, v: Value) -> u8 {
        if self.chunk().const_count() == MAX_CHUNK_CONSTANTS {
            self.error("Too many constants in one chunk");
            return MAX_CHUNK_CONSTANTS as u8;
        }
        let const_id = self.chunk().write_const(v);
        const_id as u8
    }

    fn emit(&mut self, op: OpCode) {
        let pos = self.previous_token.pos;
        self.chunk().write_instruction(op, pos);
    }

    fn emit_return(&mut self) {
        self.emit(OpCode::Nil);
        self.emit(OpCode::Return);
    }

    fn emit_jump<O: Fn(u16) -> OpCode>(&mut self, op: O) -> usize {
        self.emit(op(0xFFFF));
        self.chunk().instructions_count()
    }

    fn patch_jump(&mut self, jump: usize) {
        let offset = self.chunk().instructions_count() - jump;
        if offset > u16::MAX as usize {
            self.error_current("Too much code to jump over");
            return;
        }
        self.chunk().patch_jump_instruction(jump - 1, offset as u16);
    }

    fn emit_loop(&mut self, loop_start: usize) {
        // +1 because the offset also takes into account the newly emitted loop opcode
        let offset = self.chunk().instructions_count() - loop_start + 1;
        if offset > u16::MAX as usize {
            self.error("Loop body too large");
            return;
        }
        self.emit(OpCode::Loop(offset as u16));
    }

    fn declaration(&mut self) {
        if self.match_type(token::Type::Fun) {
            self.fun_declaration()
        } else if self.match_type(token::Type::Var) {
            self.var_declaration()
        } else {
            self.statement()
        }

        if self.panic {
            self.synchronize();
        }
    }

    fn fun_declaration(&mut self) {
        let ident_id = self.parse_variable();
        self.mark_initialized();
        self.function(FunType::Function);
        self.define_variable(ident_id);
    }

    fn function(&mut self, fun_t: FunType) {
        let name = intern::id(self.previous_token.lexeme);
        self.nestings.push(Nesting::new(
            ObjFun {
                name,
                arity: 0,
                chunk: Chunk::default(),
            },
            fun_t,
        ));
        self.begin_scope();

        self.consume(token::Type::LParen, "Expect '(' after function name");
        if !self.check(token::Type::RParen) {
            loop {
                if self.nest_mut().fun.arity as usize == MAX_PARAMS {
                    self.error_current("Can't have more than 255 parameters");
                }

                self.nest_mut().fun.arity += 1;
                let ident_id = self.parse_variable();
                self.define_variable(ident_id);

                if !self.match_type(token::Type::Comma) {
                    break;
                }
            }
        }
        self.consume(token::Type::RParen, "Expect ')' after parameters");
        self.consume(token::Type::LBrace, "Expect '{' before function body");
        self.block();

        if let Some(fun) = self.finish() {
            let fun = Rc::new(fun);
            let const_id = self.make_const(Value::Fun(fun));
            self.emit(OpCode::Constant(const_id));
        }
    }

    fn var_declaration(&mut self) {
        let ident_id = self.parse_variable();
        // initializer
        if self.match_type(token::Type::Equal) {
            self.expression();
        } else {
            self.emit(OpCode::Nil);
        }
        // ; terminated
        self.consume(
            token::Type::Semicolon,
            "Expect ';' after variable declaration",
        );
        self.define_variable(ident_id);
    }

    fn parse_variable(&mut self) -> u8 {
        self.consume(token::Type::Ident, "Expect variable name");
        self.declare_variable();
        self.identifier_constant()
    }

    fn identifier_constant(&mut self) -> u8 {
        if self.nest().scope_depth > 0 {
            0 // A dummy value used when we're not in the global scope
        } else {
            let name = intern::id(self.previous_token.lexeme);
            self.make_const(Value::String(name))
        }
    }

    fn declare_variable(&mut self) {
        if self.nest().scope_depth == 0 {
            return;
        }
        if self.nest().locals.len() == MAX_LOCAL_VARIABLES {
            self.error("Too many local variables in function");
        }

        let name = intern::id(self.previous_token.lexeme);
        let mut name_duplicated = false;
        for l in self.nest().locals.iter() {
            if l.initialized && l.depth < self.nest().scope_depth {
                break;
            }
            if l.name == name {
                name_duplicated = true;
                break;
            }
        }
        if name_duplicated {
            self.error("Already a variable with this name in this scope");
        }

        let scope_depth = self.nest().scope_depth;
        self.nest_mut().locals.push((name, scope_depth).into());
    }

    fn define_variable(&mut self, ident_id: u8) {
        // Local variables are not looked up by name. There's no need to stuff
        // the variable name into the constant table.
        if self.nest().scope_depth > 0 {
            self.mark_initialized();
        } else {
            self.emit(OpCode::DefineGlobal(ident_id));
        }
    }

    fn mark_initialized(&mut self) {
        if self.nest().scope_depth == 0 {
            return;
        }
        self.nest_mut()
            .locals
            .last_mut()
            .expect("Just pushed")
            .initialized = true;
    }

    fn statement(&mut self) {
        if self.match_type(token::Type::Print) {
            self.print_statement();
        } else if self.match_type(token::Type::For) {
            self.for_statement();
        } else if self.match_type(token::Type::If) {
            self.if_statement();
        } else if self.match_type(token::Type::Return) {
            self.return_statement();
        } else if self.match_type(token::Type::While) {
            self.while_statement();
        } else if self.match_type(token::Type::LBrace) {
            self.begin_scope();
            self.block();
            self.end_scope();
        } else {
            self.expression_statement();
        }
    }

    fn block(&mut self) {
        while !self.check(token::Type::RBrace) && !self.check(token::Type::Eof) {
            self.declaration();
        }
        self.consume(token::Type::RBrace, "Expect '}' after block");
    }

    fn begin_scope(&mut self) {
        self.nest_mut().scope_depth += 1;
    }

    fn end_scope(&mut self) {
        self.nest_mut().scope_depth -= 1;
        while let Some(l) = self.nest().locals.last() {
            if l.depth <= self.nest().scope_depth {
                break;
            }
            self.emit(OpCode::Pop);
            self.nest_mut().locals.pop();
        }
    }

    fn return_statement(&mut self) {
        if self.nest().fun_t == FunType::Script {
            self.error("Can't return from top-level code")
        }

        if self.match_type(token::Type::Semicolon) {
            self.emit_return();
        } else {
            self.expression();
            self.consume(token::Type::Semicolon, "Expect ';' after return value");
            self.emit(OpCode::Return);
        }
    }

    fn if_statement(&mut self) {
        self.consume(token::Type::LParen, "Expect '(' after 'if'");
        self.expression();
        self.consume(token::Type::RParen, "Expect ')' after condition");

        // This jumps to the else clause
        let then_jump = self.emit_jump(OpCode::JumpIfFalse);
        // Jump does not pop the conditional out of the stack, so we do it manually.
        // Here we pop the true value.
        self.emit(OpCode::Pop);
        self.statement();

        // This jumps through the else clause
        let else_jump = self.emit_jump(OpCode::Jump);
        self.patch_jump(then_jump);
        // Here we pop the false value.
        self.emit(OpCode::Pop);

        if self.match_type(token::Type::Else) {
            self.statement();
        }
        self.patch_jump(else_jump);
    }

    fn while_statement(&mut self) {
        let loop_start = self.chunk().instructions_count();
        self.consume(token::Type::LParen, "Expect '(' after 'while'");
        self.expression();
        self.consume(token::Type::RParen, "Expect ')' after condition");

        let exit_jump = self.emit_jump(OpCode::JumpIfFalse);
        self.emit(OpCode::Pop);

        self.statement();
        self.emit_loop(loop_start);

        self.patch_jump(exit_jump);
        self.emit(OpCode::Pop);
    }

    fn for_statement(&mut self) {
        self.begin_scope();
        self.consume(token::Type::LParen, "Expect '(' after 'for'");
        // initializer clause
        if self.match_type(token::Type::Semicolon) {
            // no initializer
        } else if self.match_type(token::Type::Var) {
            self.var_declaration();
        } else {
            self.expression_statement();
        }

        let mut loop_start = self.chunk().instructions_count();

        // conditional clause
        let exit_jump = if !self.match_type(token::Type::Semicolon) {
            // conditional expression
            self.expression();
            self.consume(token::Type::Semicolon, "Expect ';' after loop condition");
            // exit if consitional expression is falsey
            let exit_jump = self.emit_jump(OpCode::JumpIfFalse);
            // pop true when not jump
            self.emit(OpCode::Pop);
            Some(exit_jump)
        } else {
            None
        };

        // increment clause
        if !self.match_type(token::Type::RParen) {
            // immediately jump to the loop's body, skipping the increment expression
            let body_jump = self.emit_jump(OpCode::Jump);
            let increment_start = self.chunk().instructions_count();
            // increment expression
            self.expression();
            // pop expression result
            self.emit(OpCode::Pop);
            self.consume(token::Type::RParen, "Expect ')' after for clauses");

            // this will loop back to the conditional after the increment expression is run
            self.emit_loop(loop_start);
            // the loop start to point to the increment expression
            loop_start = increment_start;
            self.patch_jump(body_jump);
        }

        self.statement();
        // this will loop back to the increment expression if there is one, otherwise it loops back
        // to the conditional expression
        self.emit_loop(loop_start);

        if let Some(exit_jump) = exit_jump {
            self.patch_jump(exit_jump);
            // pop false when get jumped into
            self.emit(OpCode::Pop);
        }
        self.end_scope();
    }

    fn print_statement(&mut self) {
        self.expression();
        self.consume(token::Type::Semicolon, "Expect ';' after value");
        self.emit(OpCode::Print);
    }

    fn expression_statement(&mut self) {
        self.expression();
        self.consume(token::Type::Semicolon, "Expect ';' after expression");
        self.emit(OpCode::Pop);
    }

    fn expression(&mut self) {
        self.parse_precedence(Precedence::Assignment)
    }

    fn or(&mut self) {
        // Short-circuit jump.
        // If the value on top of the stack is falsey, we make a small jump skipping passs the jump
        // right beneath. Otherwise we go to the jump right beneath us to jump pass the rest of the
        // operands. This simulates JumpIfTrue without making a new opcode.
        let else_jump = self.emit_jump(OpCode::JumpIfFalse);
        let end_jump = self.emit_jump(OpCode::Jump);

        self.patch_jump(else_jump);
        // Pop false value if not short-circuited
        self.emit(OpCode::Pop);

        self.parse_precedence(Precedence::Or);
        self.patch_jump(end_jump);
    }

    fn and(&mut self) {
        // Short-circuit jump.
        // If the value on top of the stack is falsey, jumps pass the rest of the
        // operands.
        let end_jump = self.emit_jump(OpCode::JumpIfFalse);
        // Pop true value if not short-circuited
        self.emit(OpCode::Pop);

        self.parse_precedence(Precedence::And);
        self.patch_jump(end_jump);
    }

    fn binary(&mut self) {
        let token_type = self.previous_token.typ;
        self.parse_precedence(Precedence::of(token_type).next());
        match token_type {
            token::Type::BangEqual => {
                self.emit(OpCode::Equal);
                self.emit(OpCode::Not);
            }
            token::Type::EqualEqual => self.emit(OpCode::Equal),
            token::Type::Greater => self.emit(OpCode::Greater),
            token::Type::GreaterEqual => {
                self.emit(OpCode::Less);
                self.emit(OpCode::Not);
            }
            token::Type::Less => self.emit(OpCode::Less),
            token::Type::LessEqual => {
                self.emit(OpCode::Greater);
                self.emit(OpCode::Not);
            }
            token::Type::Plus => self.emit(OpCode::Add),
            token::Type::Minus => self.emit(OpCode::Subtract),
            token::Type::Star => self.emit(OpCode::Multiply),
            token::Type::Slash => self.emit(OpCode::Divide),
            _ => unreachable!("Rule table is wrong"),
        }
    }

    fn unary(&mut self) {
        let token_type = self.previous_token.typ;
        self.parse_precedence(Precedence::Unary);
        match token_type {
            token::Type::Bang => self.emit(OpCode::Not),
            token::Type::Minus => self.emit(OpCode::Negate),
            _ => unreachable!("Rule table is wrong"),
        }
    }

    fn call(&mut self) {
        let arg_count = self.argument_list();
        self.emit(OpCode::Call(arg_count));
    }

    fn argument_list(&mut self) -> u8 {
        let mut arg_count = 0;
        if !self.check(token::Type::RParen) {
            loop {
                self.expression();
                if arg_count == MAX_PARAMS {
                    self.error("Can't have more than 255 arguments");
                    return MAX_PARAMS as u8;
                }
                arg_count += 1;
                if !self.match_type(token::Type::Comma) {
                    break;
                }
            }
        }
        self.consume(token::Type::RParen, "Expect ')' after arguments");
        arg_count as u8
    }

    fn variable(&mut self, can_assign: bool) {
        let (op_get, op_set) =
            if let Some(local) = self.resolve_local(intern::id(self.previous_token.lexeme)) {
                (OpCode::GetLocal(local), OpCode::SetLocal(local))
            } else {
                let name = intern::id(self.previous_token.lexeme);
                let ident_id = self.make_const(Value::String(name));
                (OpCode::GetGlobal(ident_id), OpCode::SetGlobal(ident_id))
            };

        if can_assign && self.match_type(token::Type::Equal) {
            self.expression();
            self.emit(op_set);
        } else {
            self.emit(op_get);
        }
    }

    fn resolve_local(&mut self, name: StringId) -> Option<u8> {
        self.nest()
            .locals
            .iter()
            .enumerate()
            .rev()
            .find(|(_, l)| l.name == name)
            .map(|(i, l)| (i as u8, l.initialized))
            .map(|(i, init)| {
                if !init {
                    self.error("Can't read local variable in its own initializer");
                }
                i
            })
    }

    fn string(&mut self) {
        let value =
            intern::id(&self.previous_token.lexeme[1..self.previous_token.lexeme.len() - 1]);
        let constant = self.make_const(Value::String(value));
        self.emit(OpCode::Constant(constant));
    }

    fn number(&mut self) {
        let value = intern::str(intern::id(self.previous_token.lexeme))
            .parse()
            .expect("Validated by scanner");
        let constant = self.make_const(Value::Number(value));
        self.emit(OpCode::Constant(constant));
    }

    fn literal(&mut self) {
        match self.previous_token.typ {
            token::Type::False => self.emit(OpCode::False),
            token::Type::Nil => self.emit(OpCode::Nil),
            token::Type::True => self.emit(OpCode::True),
            _ => unreachable!("Rule table is wrong"),
        }
    }

    fn grouping(&mut self) {
        self.expression();
        self.consume(token::Type::RParen, "Expect ')' after expression");
    }

    fn parse_precedence(&mut self, precedence: Precedence) {
        self.advance();
        let can_assign = precedence <= Precedence::Assignment;
        self.prefix_rule(can_assign);

        while precedence <= Precedence::of(self.current_token.typ) {
            self.advance();
            self.infix_rule();
        }

        if can_assign && self.match_type(token::Type::Equal) {
            self.error("Invalid assignment target");
        }
    }

    fn prefix_rule(&mut self, can_assign: bool) {
        match self.previous_token.typ {
            token::Type::LParen => self.grouping(),
            token::Type::Minus | token::Type::Bang => self.unary(),
            token::Type::Ident => self.variable(can_assign),
            token::Type::String => self.string(),
            token::Type::Number => self.number(),
            token::Type::True | token::Type::False | token::Type::Nil => self.literal(),
            _ => {
                self.error("Expect expression");
            }
        }
    }

    fn infix_rule(&mut self) {
        match self.previous_token.typ {
            token::Type::LParen => self.call(),
            token::Type::Or => self.or(),
            token::Type::And => self.and(),
            token::Type::Minus
            | token::Type::Plus
            | token::Type::Slash
            | token::Type::Star
            | token::Type::BangEqual
            | token::Type::EqualEqual
            | token::Type::Greater
            | token::Type::GreaterEqual
            | token::Type::Less
            | token::Type::LessEqual => self.binary(),
            _ => self.error("Expect expression"),
        }
    }

    fn synchronize(&mut self) {
        self.panic = false;
        while !self.check(token::Type::Eof) {
            if self.previous_token.typ == token::Type::Semicolon {
                return;
            }
            match self.current_token.typ {
                token::Type::Class
                | token::Type::Fun
                | token::Type::Var
                | token::Type::For
                | token::Type::If
                | token::Type::While
                | token::Type::Print
                | token::Type::Return => return,
                _ => {}
            }
            self.advance();
        }
    }

    fn advance(&mut self) {
        loop {
            match self.scanner.scan() {
                Err(err) => {
                    eprintln!("{}", err);
                    self.had_error = true;
                }
                Ok(tok) => {
                    self.previous_token = std::mem::replace(&mut self.current_token, tok);
                    break;
                }
            }
        }
    }

    fn check(&mut self, typ: token::Type) -> bool {
        if self.current_token.typ != typ {
            return false;
        }
        true
    }

    fn match_type(&mut self, typ: token::Type) -> bool {
        if self.current_token.typ != typ {
            return false;
        }
        self.advance();
        true
    }

    fn consume(&mut self, typ: token::Type, msg: &'static str) {
        if self.current_token.typ != typ {
            self.error_current(msg);
            return;
        }
        self.advance();
    }

    fn error(&mut self, message: &'static str) {
        self.error_at(self.previous_token.pos, self.previous_token.lexeme, message)
    }

    fn error_current(&mut self, message: &'static str) {
        self.error_at(self.current_token.pos, self.current_token.lexeme, message)
    }

    fn error_at(&mut self, pos: Position, lexeme: &str, message: &'static str) {
        if self.panic {
            return;
        }
        self.had_error = true;
        self.panic = true;

        if lexeme.is_empty() {
            eprintln!("{} Error at end: {}.", pos, message)
        } else {
            eprintln!("{} Error at '{}': {}.", pos, lexeme, message)
        }
    }
}

#[derive(Debug)]
struct Nesting {
    fun: ObjFun,
    fun_t: FunType,
    locals: Vec<Local>,
    scope_depth: usize,
}

impl Nesting {
    fn new(fun: ObjFun, fun_t: FunType) -> Self {
        // The first slot on the stack is reserved for the callframe
        let mut locals = Vec::with_capacity(MAX_STACK);
        locals.push(Local {
            name: intern::id(""),
            depth: 0,
            initialized: false,
        });
        Self {
            fun,
            fun_t,
            locals,
            scope_depth: 0,
        }
    }
}

/// Store name and depth of the resolved identifer.
#[derive(Debug)]
struct Local {
    name: StringId,
    depth: usize,
    initialized: bool,
}

impl From<(StringId, usize)> for Local {
    fn from((name, depth): (StringId, usize)) -> Self {
        Self {
            name,
            depth,
            initialized: false,
        }
    }
}

/// Chunk is a sequence of instructions and data that will be written to by the compiler
/// and later run by the virtual-machine.
///
/// # Examples
/// All precedence levels in Lox
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Precedence {
    /// No precedence
    None,
    /// Operator `=`
    Assignment,
    /// Operator `or`
    Or,
    /// Operator `and`
    And,
    /// Operator `==` `!=`
    Equality,
    /// Operator `<` `>` `<=` `>=`
    Comparison,
    /// Operator `+` `-`
    Term,
    /// Operator `*` `/`
    Factor,
    /// Operator `!` `-`
    Unary,
    /// Operator `.` `()`
    Call,
    /// Literal and keywords
    Primary,
}

impl Precedence {
    /// Get the immediately higher precedence level
    fn next(&self) -> Self {
        match self {
            Self::None => Self::Assignment,
            Self::Assignment => Self::Or,
            Self::Or => Self::And,
            Self::And => Self::Equality,
            Self::Equality => Self::Comparison,
            Self::Comparison => Self::Term,
            Self::Term => Self::Factor,
            Self::Factor => Self::Unary,
            Self::Unary => Self::Call,
            Self::Call => Self::Primary,
            Self::Primary => Self::Primary,
        }
    }

    fn of(typ: token::Type) -> Self {
        match typ {
            token::Type::Or => Precedence::Or,
            token::Type::And => Precedence::And,
            token::Type::BangEqual | token::Type::EqualEqual => Precedence::Equality,
            token::Type::Greater
            | token::Type::GreaterEqual
            | token::Type::Less
            | token::Type::LessEqual => Precedence::Comparison,
            token::Type::Minus | token::Type::Plus => Precedence::Term,
            token::Type::Slash | token::Type::Star => Precedence::Factor,
            token::Type::LParen => Precedence::Call,
            _ => Self::None,
        }
    }
}
