// Package coded carries the structured errors of the Telos surface: a stable
// TELOS_* code, a human message, and optional repository paths. The CLI's
// JSON envelope serializes them; every layer underneath (kernel, contract,
// evidence) produces them without importing the CLI.
package coded

import "errors"

// Error is a structured command error.
type Error struct {
	Code    string
	Message string
	Paths   []string
}

func (e *Error) Error() string { return e.Message }

// New returns a structured error with the given code.
func New(code, message string) error {
	return &Error{Code: code, Message: message}
}

// WithPaths returns a structured error naming the repository paths involved.
func WithPaths(code, message string, paths []string) error {
	return &Error{Code: code, Message: message, Paths: paths}
}

// As extracts a structured error from err's chain.
func As(err error) (*Error, bool) {
	var e *Error
	if errors.As(err, &e) {
		return e, true
	}
	return nil, false
}
