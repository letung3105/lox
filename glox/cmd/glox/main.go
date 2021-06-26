package main

// This is an interpreter for the Lox programming language written in Go.

import (
	"bufio"
	"fmt"
	"io/ioutil"
	"os"

	gloxErrors "github.com/letung3105/lox/glox/internal/errors"
	"github.com/letung3105/lox/glox/internal/scanner"
)

func main() {
	args := os.Args[1:]
	if len(args) > 1 {
		fmt.Println("Usage: glox [script]")
		os.Exit(64)
	}

	reporter := gloxErrors.NewSimpleReporter(os.Stdout)
	if len(args) != 1 {
		runPrompt(reporter)
	} else {
		runFile(args[0], reporter)
	}

	if reporter.HadError() {
		os.Exit(65)
	}
}

func run(script string, reporter gloxErrors.Reporter) {
	sc := scanner.New([]rune(script), reporter)
	for _, tok := range sc.Scan() {
		fmt.Println(tok)
	}
}

// Run the interpreter in REPL mode
func runPrompt(reporter gloxErrors.Reporter) {
	s := bufio.NewScanner(os.Stdin)
	s.Split(bufio.ScanLines)
	for {
		fmt.Print("> ")
		if !s.Scan() {
			break
		}
		run(s.Text(), reporter)
	}
	if err := s.Err(); err != nil {
		reporter.Report(err)
	}
}

// Run the given file as script
func runFile(fpath string, reporter gloxErrors.Reporter) {
	bytes, err := ioutil.ReadFile(fpath)
	if err != nil {
		reporter.Report(err)
		return
	}
	run(string(bytes), reporter)
}