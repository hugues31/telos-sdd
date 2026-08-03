package telos

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"
)

type commandError struct {
	Code    string
	Message string
	Paths   []string
}

func (e *commandError) Error() string { return e.Message }

func coded(code, message string) error {
	return &commandError{Code: code, Message: message}
}

func codedPaths(code, message string, paths []string) error {
	return &commandError{Code: code, Message: message, Paths: paths}
}

type reportedError struct{ err error }

func (e *reportedError) Error() string { return e.err.Error() }
func (e *reportedError) Unwrap() error { return e.err }

func IsReported(err error) bool {
	var reported *reportedError
	return errors.As(err, &reported)
}

type commandExecution struct {
	Command string
	Result  any
	Next    []string
	Human   string
}

func emitJSON(stdout io.Writer, execution commandExecution) error {
	response := map[string]any{
		"ok":           true,
		"command":      execution.Command,
		"result":       execution.Result,
		"next_actions": execution.Next,
	}
	return json.NewEncoder(stdout).Encode(response)
}

func emitJSONError(stdout io.Writer, command string, err error) error {
	detail := &commandError{Code: "TELOS_COMMAND_FAILED", Message: err.Error()}
	var commandErr *commandError
	if errors.As(err, &commandErr) {
		detail = commandErr
	}
	response := map[string]any{
		"ok":      false,
		"command": command,
		"error": map[string]any{
			"code":    detail.Code,
			"message": detail.Message,
			"paths":   detail.Paths,
		},
	}
	if encodeErr := json.NewEncoder(stdout).Encode(response); encodeErr != nil {
		return fmt.Errorf("%v (encode JSON error: %w)", err, encodeErr)
	}
	return &reportedError{err: err}
}

func extractJSON(args []string) (bool, []string) {
	jsonMode := false
	clean := make([]string, 0, len(args))
	for _, arg := range args {
		if arg == "--json" {
			jsonMode = true
			continue
		}
		clean = append(clean, arg)
	}
	return jsonMode, clean
}

func commandLabel(args []string) string {
	if len(args) == 0 {
		return "help"
	}
	parts := []string{args[0]}
	if len(args) > 1 && !strings.HasPrefix(args[1], "-") {
		parts = append(parts, args[1])
	}
	return strings.Join(parts, ".")
}
