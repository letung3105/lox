package lox

import (
	"fmt"
	"io"
)

// Reporter defines the interface for structure that can display errors to the
// user. A reporter is defined to separated errors reporting code from errors
// displaying code. Fully-features languages have a complex setup for reporting
// errors to user.
type Reporter interface {
	Report(err error)
	Reset()
	HadError() bool
	HadRuntimeError() bool
}

// SimpleReporter writes error as-is to inner writer
type SimpleReporter struct {
	writer        io.Writer
	hadErr        bool
	hadRuntimeErr bool
}

func NewSimpleReporter(writer io.Writer) Reporter {
	return &SimpleReporter{writer, false, false}
}

func (reporter *SimpleReporter) Report(err error) {
	fmt.Fprintln(reporter.writer, err)
	if _, isRuntimeErr := err.(*RuntimeError); isRuntimeErr {
		reporter.hadRuntimeErr = true
	} else {
		reporter.hadErr = true
	}
}

func (reporter *SimpleReporter) Reset() {
	reporter.hadErr = false
}

func (reporter *SimpleReporter) HadError() bool {
	return reporter.hadErr
}

func (reporter *SimpleReporter) HadRuntimeError() bool {
	return reporter.hadRuntimeErr
}