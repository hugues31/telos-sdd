package telos

import (
	"bytes"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/gitx"
	"github.com/hugues31/telos-sdd/internal/kernel"
)

const usage = `Telos — certified-state development: every accepted state is verified

Usage:
  telos init [--agent codex|claude|all] [--ci github] [--json]
  telos status [--json]
  telos verify [--json]
  telos doctor [--json]
  telos guard
  telos version [--json]

The Change lifecycle (change start/review/approve/ready/promote, salvage,
evidence, findings, search, context, view) arrives milestone by milestone in
the v0.6 rewrite; see docs/design-v2.md.`

// Run dispatches the CLI. Every command supports the stable JSON envelope
// {ok, command, result, next_actions, error{code,message,paths}}.
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
		execution, err := runInit(cwd, version, args[1:], stderr)
		return finish(execution, err)
	}
	root, err := findRoot(cwd)
	if err != nil {
		return finish(commandExecution{}, coded.New("TELOS_NOT_INITIALIZED", err.Error()))
	}
	repo, err := gitx.Open(root)
	if err != nil {
		return finish(commandExecution{}, coded.New("TELOS_GIT_REPOSITORY_REQUIRED", err.Error()))
	}

	switch args[0] {
	case "doctor":
		return finish(commandExecution{}, runDoctor(repo, commandStdout))
	case "status":
		st, err := kernel.Status(repo)
		human := ""
		var next []string
		if err == nil {
			human = fmt.Sprintf("State: %s.", st.State)
			switch st.State {
			case kernel.StateUninitialized:
				next = []string{"init"}
			case kernel.StateCorrupted:
				human += " " + st.Reason
			}
		}
		return finish(commandExecution{Command: "status", Result: st, Next: next, Human: human}, err)
	case "verify":
		cfg, err := kernel.ReadConfig(root)
		if err != nil {
			return finish(commandExecution{}, err)
		}
		report, err := kernel.Verify(repo, cfg, commandStdout, stderr)
		human := ""
		if err == nil {
			human = fmt.Sprintf("Verified: certificate %s sealed, %d requirement(s), worktree matches the certified state.", report.Change, report.Requirements)
		}
		return finish(commandExecution{Command: "verify", Result: report, Human: human}, err)
	default:
		return finish(commandExecution{}, coded.New("TELOS_INPUT_INVALID", fmt.Sprintf("unknown command %q; run `telos help`", args[0])))
	}
}

func flags(name string, stderr io.Writer) *flag.FlagSet {
	f := flag.NewFlagSet(name, flag.ContinueOnError)
	f.SetOutput(stderr)
	return f
}

func runInit(cwd, version string, args []string, stderr io.Writer) (commandExecution, error) {
	f := flags("init", stderr)
	agent := f.String("agent", "all", "agent integration")
	ci := f.String("ci", "", "CI integration")
	if err := f.Parse(args); err != nil {
		return commandExecution{}, err
	}
	if f.NArg() != 0 {
		return commandExecution{}, coded.New("TELOS_INPUT_INVALID", "init takes no positional arguments")
	}
	if *ci != "" && *ci != "github" {
		return commandExecution{}, coded.New("TELOS_INPUT_INVALID", fmt.Sprintf("unsupported CI %q", *ci))
	}
	if *agent != "codex" && *agent != "claude" && *agent != "all" {
		return commandExecution{}, coded.New("TELOS_INPUT_INVALID", fmt.Sprintf("invalid agent %q", *agent))
	}
	if !gitx.Available() {
		return commandExecution{}, coded.New("TELOS_GIT_UNAVAILABLE", "Git is required; install Git before running `telos init`")
	}
	if !gitx.IsRepo(cwd) {
		return commandExecution{}, coded.New("TELOS_GIT_REPOSITORY_REQUIRED", "not a Git repository; run `git init` before `telos init`")
	}
	repo, err := gitx.Open(cwd)
	if err != nil {
		return commandExecution{}, err
	}
	if err := initProject(repo.WorkDir, *agent, *ci == "github"); err != nil {
		return commandExecution{}, err
	}
	cfg, err := kernel.ReadConfig(repo.WorkDir)
	if err != nil {
		return commandExecution{}, err
	}
	cert, err := kernel.Genesis(repo, cfg, kernel.GenesisOptions{Version: version})
	if err != nil {
		return commandExecution{}, err
	}
	result := map[string]any{"root": repo.WorkDir, "commit": cert.Payload.Commit, "change": cert.Payload.Change.ID}
	human := fmt.Sprintf("Initialized Telos in %s: genesis certificate sealed on %s.\nThe contract lives in spec/; tell your coding agent what you want to build.", repo.WorkDir, cert.Payload.Commit[:12])
	return commandExecution{Command: "init", Result: result, Next: []string{"status"}, Human: human}, nil
}

func runDoctor(repo *gitx.Repo, stdout io.Writer) error {
	type check struct {
		Name string
		Err  error
	}
	cfg, cfgErr := kernel.ReadConfig(repo.WorkDir)
	var certErr error
	st, stErr := kernel.Status(repo)
	switch {
	case stErr != nil:
		certErr = stErr
	case st.State != kernel.StateCertified:
		certErr = errors.New(st.State + ": " + st.Reason)
	}
	checks := []check{
		{"Telos config", cfgErr},
		{"Git", func() error {
			if !gitx.Available() {
				return errors.New("git not found")
			}
			return nil
		}()},
		{"Product spec", fileExists(filepath.Join(repo.WorkDir, filepath.FromSlash(contract.ProductFile)))},
		{"Certificate", certErr},
	}
	for _, agent := range cfg.Agents {
		switch agent {
		case "codex":
			checks = append(checks, check{"Codex Skill", fileExists(filepath.Join(repo.WorkDir, ".agents", "skills", "telos", "SKILL.md"))})
		case "claude":
			checks = append(checks, check{"Claude Skill", fileExists(filepath.Join(repo.WorkDir, ".claude", "skills", "telos", "SKILL.md"))})
		}
	}
	failed := false
	for _, c := range checks {
		status := "ok"
		if c.Err != nil {
			status, failed = c.Err.Error(), true
		}
		fmt.Fprintf(stdout, "%-14s %s\n", c.Name+":", status)
	}
	if failed {
		return errors.New("doctor found configuration errors")
	}
	return nil
}

func fileExists(path string) error { _, err := os.Stat(path); return err }

// runGuard is the provider PreToolUse hook endpoint. It is deliberately
// fail-open on malformed input and outside Telos projects: a crashing guard
// must never brick the agent. Silence is allow.
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
	if strings.EqualFold(toolName, "Bash") || strings.EqualFold(toolName, "Shell") {
		var toolInput map[string]any
		_ = json.Unmarshal(raw, &toolInput)
		command, _ := toolInput["command"].(string)
		if isTelosBrokerCommand(command) {
			return gateBrokerCommand(root, stdout, command)
		}
		return denyGuard(stdout, "Telos strict mode permits shell execution only through the Telos CLI broker.")
	}
	if strings.EqualFold(toolName, "Edit") || strings.EqualFold(toolName, "Write") || strings.EqualFold(toolName, "apply_patch") {
		return denyGuard(stdout, "Telos strict mode denies direct repository writes; use the Telos CLI broker.")
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

// gateBrokerCommand screens an already-validated broker command for the
// human-gated operations. At M1 the only gate is re-initialization (a fresh
// genesis adopting the current tree); the Change-lifecycle gates (approve,
// adopt, restore, abort) return with their commands.
func gateBrokerCommand(root string, stdout io.Writer, command string) error {
	fields := brokerCommandFields(command)
	if len(fields) < 2 {
		return nil
	}
	if fields[1] == "init" {
		return askGuard(stdout, "Telos human gate — re-initialize Telos in an already-initialized project: this seals a NEW genesis certificate adopting the current tree as-is, outside any verified transition. Approve only if you deliberately want a destructive reset of the certified state.")
	}
	return nil
}

// brokerCommandFields returns the whitespace-separated fields of the
// command's first line, excluding any heredoc redirection already validated
// by isTelosBrokerCommand.
func brokerCommandFields(command string) []string {
	firstLine := strings.TrimSpace(strings.Split(strings.ReplaceAll(command, "\r\n", "\n"), "\n")[0])
	if heredoc := strings.Index(firstLine, "<<"); heredoc >= 0 {
		firstLine = firstLine[:heredoc]
	}
	return strings.Fields(firstLine)
}

func askGuard(stdout io.Writer, reason string) error {
	response := map[string]any{"hookSpecificOutput": map[string]any{"hookEventName": "PreToolUse", "permissionDecision": "ask", "permissionDecisionReason": reason}}
	return json.NewEncoder(stdout).Encode(response)
}

func denyGuard(stdout io.Writer, reason string) error {
	response := map[string]any{"hookSpecificOutput": map[string]any{"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason}}
	return json.NewEncoder(stdout).Encode(response)
}
