package lox

import (
	"fmt"
	"io"
)

// callable is implemented by Lox's objects that can be called.
type callable interface {
	arity() int
	call(in *Interpreter, args []interface{}) (interface{}, error)
}

// Interpreter exposes methods for evaluating then given Lox syntax tree. This
// struct implements ExprVisitor
type Interpreter struct {
	globals     *environment
	environment *environment
	locals      map[Expr]int
	output      io.Writer
	reporter    Reporter
	isREPL      bool
}

func NewInterpreter(output io.Writer, reporter Reporter, isREPL bool) *Interpreter {
	env := newEnvironment(nil)
	env.define("clock", new(functionClock))

	interpreter := new(Interpreter)
	interpreter.globals = env
	interpreter.environment = env
	interpreter.locals = make(map[Expr]int)
	interpreter.output = output
	interpreter.reporter = reporter
	interpreter.isREPL = isREPL
	return interpreter
}

func (in *Interpreter) Interpret(statements []Stmt) {
	for _, stmt := range statements {
		if _, err := in.exec(stmt); err != nil {
			in.reporter.Report(err)
			break
		}
	}
}

func (in *Interpreter) VisitBlockStmt(stmt *BlockStmt) (interface{}, error) {
	return nil, in.execBlock(stmt.Stmts, newEnvironment(in.environment))
}

func (in *Interpreter) VisitExprStmt(stmt *ExprStmt) (interface{}, error) {
	expr, err := in.eval(stmt.Expr)
	if err != nil {
		return nil, err
	}
	if in.isREPL {
		switch stmt.Expr.(type) {
		case *AssignExpr, *CallExpr:
			/* expressions of these types are not printed */
		default:
			fmt.Fprintln(in.output, stringify(expr))
		}
	}
	return nil, nil
}

func (in *Interpreter) VisitClassStmt(stmt *ClassStmt) (interface{}, error) {
	var super *class
	if stmt.Super != nil {
		superObj, err := in.eval(stmt.Super)
		if err != nil {
			return nil, err
		}

		var isClass bool
		super, isClass = superObj.(*class)
		if !isClass {
			return nil, newRuntimeError(stmt.Super.Name,
				"Superclass must be a class.")
		}

		// This env holds a references to the superclass of this class,
		// the reference will never change. Any method give out by the subclass
		// will have this env attached to its closure.
		in.environment = newEnvironment(in.environment)
		in.environment.define("super", super)
	}

	methods := make(map[string]*function)
	for _, method := range stmt.Methods {
		isInitializer := method.Name.Lexeme == "init"
		fn := newFunction(method, in.environment, isInitializer)
		methods[method.Name.Lexeme] = fn
	}
	class := newClass(stmt.Name.Lexeme, super, methods)
	if super != nil {
		// pop the environment for superclass
		in.environment = in.environment.enclosing
	}
	in.environment.define(stmt.Name.Lexeme, class)
	return nil, nil
}

func (in *Interpreter) VisitFunctionStmt(stmt *FunctionStmt) (interface{}, error) {
	fn := newFunction(stmt, in.environment, false)
	in.environment.define(stmt.Name.Lexeme, fn)
	return nil, nil
}

func (in *Interpreter) VisitIfStmt(stmt *IfStmt) (interface{}, error) {
	cond, err := in.eval(stmt.Cond)
	if err != nil {
		return nil, err
	}
	if truthy(cond) {
		return in.exec(stmt.ThenBranch)
	} else if stmt.ElseBranch != nil {
		return in.exec(stmt.ElseBranch)
	}
	return nil, nil
}

func (in *Interpreter) VisitPrintStmt(stmt *PrintStmt) (interface{}, error) {
	expr, err := in.eval(stmt.Expr)
	if err != nil {
		return nil, err
	}
	fmt.Fprintln(in.output, stringify(expr))
	return nil, nil
}

func (in *Interpreter) VisitVarStmt(stmt *VarStmt) (interface{}, error) {
	var initVal interface{}
	if stmt.Init != nil {
		var err error
		initVal, err = in.eval(stmt.Init)
		if err != nil {
			return nil, err
		}
	}
	in.environment.define(stmt.Name.Lexeme, initVal)
	return nil, nil
}

func (in *Interpreter) VisitReturnStmt(stmt *ReturnStmt) (interface{}, error) {
	var val interface{}
	var err error
	if stmt.Val != nil {
		val, err = in.eval(stmt.Val)
		if err != nil {
			return nil, err
		}
	}
	return nil, newCallReturn(val)
}

func (in *Interpreter) VisitWhileStmt(stmt *WhileStmt) (interface{}, error) {
	for {
		cond, err := in.eval(stmt.Cond)
		if err != nil {
			return nil, err
		}
		if !truthy(cond) {
			return nil, nil
		}
		_, err = in.exec(stmt.Body)
		if err != nil {
			return nil, err
		}
	}
}

func (in *Interpreter) VisitAssignExpr(expr *AssignExpr) (interface{}, error) {
	val, err := in.eval(expr.Val)
	if err != nil {
		return nil, err
	}

	if steps, ok := in.locals[expr]; ok {
		in.environment.assignAt(steps, expr.Name, val)
		return val, nil
	} else {
		return val, in.globals.assign(expr.Name, val)
	}
}

func (in *Interpreter) VisitBinaryExpr(expr *BinaryExpr) (interface{}, error) {
	lhs, err := in.eval(expr.Lhs)
	if err != nil {
		return nil, err
	}
	rhs, err := in.eval(expr.Rhs)
	if err != nil {
		return nil, err
	}

	switch expr.Op.Type {
	case BANG_EQUAL:
		result := lhs != rhs
		return result, nil

	case EQUAL_EQUAL:
		result := lhs == rhs
		return result, nil

	case GREATER:
		leftNum, okLeftNum := lhs.(float64)
		rightNum, okRightNum := rhs.(float64)
		if okLeftNum && okRightNum {
			result := leftNum > rightNum
			return result, nil
		}
		return nil, newRuntimeError(expr.Op, "Operands must be numbers.")

	case GREATER_EQUAL:
		leftNum, okLeftNum := lhs.(float64)
		rightNum, okRightNum := rhs.(float64)
		if okLeftNum && okRightNum {
			result := leftNum >= rightNum
			return result, nil
		}
		return nil, newRuntimeError(expr.Op, "Operands must be numbers.")

	case LESS:
		leftNum, okLeftNum := lhs.(float64)
		rightNum, okRightNum := rhs.(float64)
		if okLeftNum && okRightNum {
			result := leftNum < rightNum
			return result, nil
		}
		return nil, newRuntimeError(expr.Op, "Operands must be numbers.")

	case LESS_EQUAL:
		leftNum, okLeftNum := lhs.(float64)
		rightNum, okRightNum := rhs.(float64)
		if okLeftNum && okRightNum {
			result := leftNum <= rightNum
			return result, nil
		}
		return nil, newRuntimeError(expr.Op, "Operands must be numbers.")

	case MINUS:
		leftNum, okLeftNum := lhs.(float64)
		rightNum, okRightNum := rhs.(float64)
		if okLeftNum && okRightNum {
			result := leftNum - rightNum
			return result, nil
		}
		return nil, newRuntimeError(expr.Op, "Operands must be numbers.")

	case PLUS:
		leftStr, okLeftStr := lhs.(string)
		rightStr, okRightStr := rhs.(string)
		if okLeftStr && okRightStr {
			result := leftStr + rightStr
			return result, nil
		}
		leftNum, okLeftNum := lhs.(float64)
		rightNum, okRightNum := rhs.(float64)
		if okLeftNum && okRightNum {
			result := leftNum + rightNum
			return result, nil
		}

		return nil, newRuntimeError(expr.Op, "Operands must be two numbers or two strings.")

	case SLASH:
		leftNum, okLeftNum := lhs.(float64)
		rightNum, okRightNum := rhs.(float64)
		if okLeftNum && okRightNum {
			result := leftNum / rightNum
			return result, nil
		}
		return nil, newRuntimeError(expr.Op, "Operands must be numbers.")

	case STAR:
		leftNum, okLeftNum := lhs.(float64)
		rightNum, okRightNum := rhs.(float64)
		if okLeftNum && okRightNum {
			result := leftNum * rightNum
			return result, nil
		}
		return nil, newRuntimeError(expr.Op, "Operands must be numbers.")
	}
	panic("Unreachable")
}

func (in *Interpreter) VisitCallExpr(expr *CallExpr) (interface{}, error) {
	callee, err := in.eval(expr.Callee)
	if err != nil {
		return nil, err
	}

	/*
		NOTE: Here we evaluate each expressions in order. This is a subtle semantic
		choice. The order in which these arguments are evaluated could be user
		visible, because expressions can have side-effects. Languages like C and Scheme
		don't specify an order, so compilers can freely rearrange them for efficiency,
		but users may be unpleasantly surprised if arguments aren't evaluated in the
		order they expected.
	*/
	var args []interface{}
	for _, arg := range expr.Args {
		argVal, err := in.eval(arg)
		if err != nil {
			return nil, err
		}
		args = append(args, argVal)
	}

	call, isCallable := callee.(callable)
	if !isCallable {
		return nil, newRuntimeError(expr.Paren, "Can only call functions and classes.")
	}
	/*
		NOTE: The arity check could be done within the Call() method. But we have lots
		of different Lox's objects that can be called, resulting in the check has to
		be reimplemented by each object. We only has to do it once, if it's performed
		here.
	*/
	if len(args) != call.arity() {
		return nil, newRuntimeError(expr.Paren, fmt.Sprintf(
			"Expected %d arguments but got %d.", call.arity(), len(args),
		))
	}
	return call.call(in, args)
}

func (in *Interpreter) VisitGetExpr(expr *GetExpr) (interface{}, error) {
	obj, err := in.eval(expr.Obj)
	if err != nil {
		return nil, err
	}

	if inst, ok := obj.(*instance); ok {
		return inst.get(expr.Name)
	} else {
		return nil, newRuntimeError(expr.Name, "Only instances have properties.")
	}
}

func (in *Interpreter) VisitGroupExpr(expr *GroupExpr) (interface{}, error) {
	return in.eval(expr.Expr)
}

func (in *Interpreter) VisitLiteralExpr(expr *LiteralExpr) (interface{}, error) {
	return expr.Val, nil
}

func (in *Interpreter) VisitLogicalExpr(expr *LogicalExpr) (interface{}, error) {
	lhs, err := in.eval(expr.Lhs)
	if err != nil {
		return nil, err
	}

	switch expr.Op.Type {
	case OR:
		if truthy(lhs) {
			return lhs, nil
		}
	case AND:
		if !truthy(lhs) {
			return lhs, nil
		}
	default:
		panic("Unreachable")
	}

	return in.eval(expr.Rhs)
}

func (in *Interpreter) VisitSetExpr(expr *SetExpr) (interface{}, error) {
	obj, err := in.eval(expr.Obj)
	if err != nil {
		return nil, err
	}

	if obj, ok := obj.(*instance); ok {
		val, err := in.eval(expr.Val)
		if err != nil {
			return nil, err
		}
		obj.set(expr.Name, val)
		return val, nil
	} else {
		return nil, newRuntimeError(expr.Name, "Only instances have fields.")
	}
}

func (in *Interpreter) VisitSuperExpr(expr *SuperExpr) (interface{}, error) {
	/*
	  In a return expression, there's no convient node for the resolver to hang
	  the resolution steps to `this`. But we know that the environment that contains
	  `this` is always enclosed by the environment that contains `super`.
	*/
	steps := in.locals[expr]
	super := in.environment.getAt(steps, "super").(*class)
	this := in.environment.getAt(steps-1, "this").(*instance)
	method, hasMethod := super.findMethod(expr.Method.Lexeme)
	if !hasMethod {
		return nil, newRuntimeError(expr.Method, fmt.Sprintf(
			"Undefined property '%s'.", expr.Method.Lexeme,
		))
	}
	return method.bind(this), nil
}

func (in *Interpreter) VisitThisExpr(expr *ThisExpr) (interface{}, error) {
	return in.lookUpVar(expr.Keyword, expr)
}

func (in *Interpreter) VisitUnaryExpr(expr *UnaryExpr) (interface{}, error) {
	exprVal, err := in.eval(expr.Expr)
	if err != nil {
		return nil, err
	}

	switch expr.Op.Type {
	case BANG:
		return !truthy(exprVal), nil
	case MINUS:
		if exprNum, ok := exprVal.(float64); ok {
			return -exprNum, nil
		}
		return nil, newRuntimeError(expr.Op, "Operand must be a number.")
	}
	panic("Unreachable")
}

func (in *Interpreter) VisitVarExpr(expr *VarExpr) (interface{}, error) {
	return in.lookUpVar(expr.Name, expr)
}

func (in *Interpreter) execBlock(statements []Stmt, env *environment) error {
	prevEnv := in.environment
	in.environment = env
	defer func() {
		in.environment = prevEnv
	}()
	for _, stmt := range statements {
		if _, err := in.exec(stmt); err != nil {
			return err
		}
	}
	return nil
}

func (in *Interpreter) exec(stmt Stmt) (interface{}, error) {
	return stmt.Accept(in)
}

func (in *Interpreter) eval(expr Expr) (interface{}, error) {
	return expr.Accept(in)
}

func (in *Interpreter) resolve(expr Expr, steps int) {
	in.locals[expr] = steps
}

func (in *Interpreter) lookUpVar(name *Token, expr Expr) (interface{}, error) {
	if steps, ok := in.locals[expr]; ok {
		return in.environment.getAt(steps, name.Lexeme), nil
	} else {
		return in.globals.get(name)
	}
}
