package telos

import (
	"bytes"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

const usage = `Telos SDD — agent-first executable intent integrity

Usage:
  telos init [--agent codex|claude|all] [--ci github] [--json]
  telos doctor [--json]
  telos inspect [--json]
  telos flow start [--request text] [--brainstorm none|choose|recommend|random|progressive] [--json]
  telos artifact put --id <id> [--json] < body.md
  telos artifact revise --id <id> [--reason text] [--json]
  telos intent new --flow <id> [--title text] [--json]
  telos intent review --flow <id> [--json]
  telos intent seal --flow <id> --review <digest> [--json]
  telos spec new --flow <id> [--title text] [--json]
  telos test-plan put --spec <id> [--json] < plan.json
  telos contract validate|review --flow <id> [--json]
  telos contract seal --flow <id> --review <digest> [--json]
  telos change begin --flow <id> [--json]
  telos change apply --flow <id> --rule <id> --scenario <id> [--json] < patch.diff
  telos change abort --flow <id> --reason <text> [--json]
  telos verify [--flow <id>] --check-only [--json]
  telos change complete --flow <id> [--json] < evidence.txt
  telos repair [--restore] [--json]
  telos guard
  telos version [--json]`

func Run(args []string, version string, stdin io.Reader, stdout, stderr io.Writer) error {
	jsonMode, args := extractJSON(args)
	label := commandLabel(args)
	var captured bytes.Buffer
	commandStdout := stdout
	if jsonMode {
		commandStdout = &captured
	}
	finish := func(execution commandExecution, err error) error {
		if err != nil {
			if jsonMode {
				return emitJSONError(stdout, label, err)
			}
			return err
		}
		if execution.Command == "" {
			execution.Command = label
		}
		if execution.Result == nil {
			execution.Result = map[string]any{"output": strings.TrimSpace(captured.String())}
		}
		if jsonMode {
			return emitJSON(stdout, execution)
		}
		if execution.Human != "" {
			fmt.Fprintln(stdout, execution.Human)
		}
		return nil
	}

	if len(args) == 0 || args[0] == "help" || args[0] == "--help" || args[0] == "-h" {
		fmt.Fprintln(commandStdout, usage)
		return finish(commandExecution{}, nil)
	}
	if args[0] == "version" {
		if jsonMode {
			return finish(commandExecution{Command: "version", Result: map[string]any{"version": version}}, nil)
		}
		fmt.Fprintln(stdout, version)
		return nil
	}
	if args[0] == "guard" {
		return runGuard(stdin, stdout)
	}
	cwd, err := os.Getwd()
	if err != nil {
		return finish(commandExecution{}, err)
	}
	if args[0] == "init" {
		err := runInit(cwd, args[1:], commandStdout, stderr)
		return finish(commandExecution{}, err)
	}
	root, err := findRoot(cwd)
	if err != nil {
		return finish(commandExecution{}, err)
	}
	if args[0] != "repair" {
		if err := requireRepositoryClean(root); err != nil {
			return finish(commandExecution{}, err)
		}
		if flow, flowErr := activeFlow(root); flowErr == nil {
			if err := auditFlowDrafts(root, flow); err != nil {
				return finish(commandExecution{}, err)
			}
			if flow.Change != "" {
				change, err := resolveChange(root, flow.ID, "")
				if err != nil {
					return finish(commandExecution{}, err)
				}
				if err := auditChangeTransactions(root, change); err != nil {
					return finish(commandExecution{}, err)
				}
			}
		} else if !errors.Is(flowErr, os.ErrNotExist) {
			return finish(commandExecution{}, flowErr)
		}
	}
	if args[0] == "doctor" {
		err := runDoctor(root, commandStdout)
		return finish(commandExecution{}, err)
	}
	execution, err := runCommand(root, args, stdin, commandStdout, stderr)
	return finish(execution, err)
}

func flags(name string, stderr io.Writer) *flag.FlagSet {
	f := flag.NewFlagSet(name, flag.ContinueOnError)
	f.SetOutput(stderr)
	return f
}

func runInit(cwd string, args []string, stdout, stderr io.Writer) error {
	f := flags("init", stderr)
	agent := f.String("agent", "all", "agent integration")
	ci := f.String("ci", "", "CI integration")
	if err := f.Parse(args); err != nil {
		return err
	}
	if f.NArg() != 0 {
		return errors.New("init takes no positional arguments")
	}
	if *ci != "" && *ci != "github" {
		return fmt.Errorf("unsupported CI %q", *ci)
	}
	if err := initProject(cwd, *agent, *ci == "github"); err != nil {
		return err
	}
	fmt.Fprintf(stdout, "Initialized Telos SDD in %s for %s.\nTell your coding agent what you want to build; it will invoke Telos automatically.\n", cwd, *agent)
	return nil
}

func runDoctor(root string, stdout io.Writer) error {
	checks := []struct {
		Name string
		Err  error
	}{
		{"Telos config", fileExists(filepath.Join(root, ".telos", "config.toml"))},
		{"Repository lock", fileExists(filepath.Join(root, filepath.FromSlash(repositoryLockPath)))},
		{"Git", commandExists("git")},
	}
	cfg, cfgErr := readConfig(root)
	checks = append(checks, struct {
		Name string
		Err  error
	}{"Config version", cfgErr})
	for _, agent := range cfg.Agents {
		switch agent {
		case "codex":
			checks = append(checks, struct {
				Name string
				Err  error
			}{"Codex Skill", fileExists(filepath.Join(root, ".agents", "skills", "telos", "SKILL.md"))})
		case "claude":
			checks = append(checks, struct {
				Name string
				Err  error
			}{"Claude Skill", fileExists(filepath.Join(root, ".claude", "skills", "telos", "SKILL.md"))})
		}
	}
	failed := false
	for _, check := range checks {
		status := "ok"
		if check.Err != nil {
			status, failed = check.Err.Error(), true
		}
		fmt.Fprintf(stdout, "%-18s %s\n", check.Name+":", status)
	}
	if failed {
		return errors.New("doctor found configuration errors")
	}
	return nil
}

func fileExists(path string) error    { _, err := os.Stat(path); return err }
func commandExists(name string) error { _, err := exec.LookPath(name); return err }
func empty(value, fallback string) string {
	if value == "" {
		return fallback
	}
	return value
}

func runGuard(stdin io.Reader, stdout io.Writer) error {
	var input map[string]any
	if err := json.NewDecoder(stdin).Decode(&input); err != nil {
		return nil
	}
	cwd, _ := input["cwd"].(string)
	if cwd == "" {
		cwd, _ = os.Getwd()
	}
	root, err := findRoot(cwd)
	if err != nil {
		return nil
	}
	toolName, _ := input["tool_name"].(string)
	raw, _ := json.Marshal(input["tool_input"])
	probe := filepath.ToSlash(string(raw))
	if strings.EqualFold(toolName, "Bash") || strings.EqualFold(toolName, "Shell") {
		var toolInput map[string]any
		_ = json.Unmarshal(raw, &toolInput)
		command, _ := toolInput["command"].(string)
		if isTelosBrokerCommand(command) {
			return nil
		}
		return denyGuard(stdout, "Telos strict mode permits shell execution only through the Telos CLI broker.")
	}
	if strings.EqualFold(toolName, "Edit") || strings.EqualFold(toolName, "Write") || strings.EqualFold(toolName, "apply_patch") {
		return denyGuard(stdout, "Telos strict mode denies direct repository writes; use the Telos CLI broker.")
	}
	lock, err := loadLock(root)
	if err != nil {
		return err
	}
	for _, file := range lock.Artifacts {
		path := filepath.ToSlash(file.Path)
		abs := filepath.ToSlash(filepath.Join(root, filepath.FromSlash(file.Path)))
		if strings.Contains(probe, path) || strings.Contains(probe, abs) {
			return denyGuard(stdout, "Telos sealed artifact is immutable: "+file.Path)
		}
	}
	return nil
}

func isTelosBrokerCommand(command string) bool {
	lines := strings.Split(strings.ReplaceAll(command, "\r\n", "\n"), "\n")
	firstLine := strings.TrimSpace(lines[0])
	fields := strings.Fields(firstLine)
	if len(fields) == 0 {
		return false
	}
	binary := filepath.Base(strings.ReplaceAll(strings.Trim(fields[0], `"'`), `\`, "/"))
	if binary != "telos" && binary != "telos.exe" {
		return false
	}
	heredoc := strings.Index(firstLine, "<<")
	probe := firstLine
	if heredoc >= 0 {
		delimiterText := strings.TrimSpace(firstLine[heredoc+2:])
		delimiterFields := strings.Fields(delimiterText)
		if len(delimiterFields) != 1 || len(lines) < 3 {
			return false
		}
		delimiter := strings.Trim(delimiterFields[0], `"'`)
		if delimiter == "" || strings.TrimSpace(lines[len(lines)-1]) != delimiter {
			return false
		}
		probe = firstLine[:heredoc]
	} else if len(lines) != 1 {
		return false
	}
	for _, operator := range []string{";", "&&", "||", "|", ">", "<", "`", "$"} {
		if strings.Contains(probe, operator) {
			return false
		}
	}
	return true
}

func denyGuard(stdout io.Writer, reason string) error {
	response := map[string]any{"hookSpecificOutput": map[string]any{"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason}}
	return json.NewEncoder(stdout).Encode(response)
}
