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
	if *agent != "codex" && *agent != "claude" && *agent != "all" {
		return fmt.Errorf("invalid agent %q", *agent)
	}
	if err := requireGitWorktree(cwd); err != nil {
		return err
	}
	if err := initProject(cwd, *agent, *ci == "github"); err != nil {
		return err
	}
	fmt.Fprintf(stdout, "Initialized Telos SDD in %s for %s.\nTell your coding agent what you want to build; it will invoke Telos automatically.\n", cwd, *agent)
	return nil
}

func requireGitWorktree(root string) error {
	git, err := exec.LookPath("git")
	if err != nil {
		return coded("TELOS_GIT_UNAVAILABLE", "Git is required; install Git before running `telos init`")
	}
	out, err := exec.Command(git, "-C", root, "rev-parse", "--is-inside-work-tree").Output()
	if err != nil || strings.TrimSpace(string(out)) != "true" {
		return coded("TELOS_GIT_REPOSITORY_REQUIRED", "not a Git repository; run `git init` before `telos init`")
	}
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
			return gateBrokerCommand(root, stdout, command)
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

// gateBrokerCommand screens an already-validated broker command for the four
// human-gate operations: intent seal, contract seal, change complete, and
// repair --restore. These never pass silently — the human decision happens at
// the provider permission prompt, not on the orchestrator's word. A seal whose
// digest no longer matches the recorded review is denied outright so the user
// is only prompted for seals that can succeed.
func gateBrokerCommand(root string, stdout io.Writer, command string) error {
	fields := brokerCommandFields(command)
	if len(fields) < 2 {
		return nil
	}
	verb := ""
	if len(fields) > 2 {
		verb = fields[2]
	}
	switch {
	case fields[1] == "intent" && verb == "seal":
		return gateIntentSeal(root, stdout, fields)
	case fields[1] == "contract" && verb == "seal":
		return gateContractSeal(root, stdout, fields)
	case fields[1] == "change" && verb == "complete":
		return gateChangeComplete(root, stdout, fields)
	case fields[1] == "repair" && hasFlag(fields, "--restore"):
		return gateRepairRestore(root, stdout)
	}
	return nil
}

func gateIntentSeal(root string, stdout io.Writer, fields []string) error {
	flow, err := loadFlow(root, flagValue(fields, "--flow"))
	digest := flagValue(fields, "--review")
	if err != nil || flow.IntentReview == "" || digest != flow.IntentReview {
		return denyGuard(stdout, "Telos human gate: the intent seal digest is missing or stale; run telos intent review and present the returned content to the user before sealing.")
	}
	title := ""
	if _, _, body, err := findArtifact(root, "intent", flow.Intent); err == nil {
		title = artifactTitle(body)
	}
	return askGuard(stdout, fmt.Sprintf("Telos human gate — seal intent %s%s for flow %s under review digest %s. Approve only if this exact intent was presented to you and is the desired outcome.", flow.Intent, quoted(title), flow.ID, shortHash(digest)))
}

func gateContractSeal(root string, stdout io.Writer, fields []string) error {
	flow, err := loadFlow(root, flagValue(fields, "--flow"))
	digest := flagValue(fields, "--review")
	if err != nil || flow.ContractReview == "" || digest != flow.ContractReview {
		return denyGuard(stdout, "Telos human gate: the contract seal digest is missing or stale; run telos contract review and present the returned content to the user before sealing.")
	}
	return askGuard(stdout, fmt.Sprintf("Telos human gate — atomically seal the executable contract for flow %s (specs %s) under review digest %s. Approve only if the presented rules, scenarios, and coverage decisions are exactly the expected behavior.", flow.ID, strings.Join(flow.Specs, ", "), shortHash(digest)))
}

func gateChangeComplete(root string, stdout io.Writer, fields []string) error {
	change, err := resolveChange(root, flagValue(fields, "--flow"), flagValue(fields, "--change"))
	if err != nil {
		return denyGuard(stdout, "Telos human gate: no change can be resolved for completion; run telos inspect --json.")
	}
	return askGuard(stdout, fmt.Sprintf("Telos human gate — complete change %s and close flow %s with independent verifier evidence. Approve only after the independent verifier reported a verified verdict.", change.ID, change.Flow))
}

func gateRepairRestore(root string, stdout io.Writer) error {
	reason := "Telos human gate — restore the repository to the last declared state. Every undeclared edit will be discarded."
	if changed, _, _, err := auditRepository(root); err == nil && len(changed) > 0 {
		preview := changed
		if len(preview) > 5 {
			preview = append(append([]string{}, changed[:5]...), fmt.Sprintf("… %d more", len(changed)-5))
		}
		reason = fmt.Sprintf("Telos human gate — restore %d undeclared path(s) to the last declared repository state, discarding their current content: %s.", len(changed), strings.Join(preview, ", "))
	}
	return askGuard(stdout, reason)
}

// brokerCommandFields returns the whitespace-separated fields of the command's
// first line, excluding any heredoc redirection already validated by
// isTelosBrokerCommand.
func brokerCommandFields(command string) []string {
	firstLine := strings.TrimSpace(strings.Split(strings.ReplaceAll(command, "\r\n", "\n"), "\n")[0])
	if heredoc := strings.Index(firstLine, "<<"); heredoc >= 0 {
		firstLine = firstLine[:heredoc]
	}
	return strings.Fields(firstLine)
}

func flagValue(fields []string, name string) string {
	for i, field := range fields {
		if field == name && i+1 < len(fields) {
			return strings.Trim(fields[i+1], `"'`)
		}
		if value, ok := strings.CutPrefix(field, name+"="); ok {
			return strings.Trim(value, `"'`)
		}
	}
	return ""
}

func hasFlag(fields []string, name string) bool {
	for _, field := range fields {
		if field == name || strings.HasPrefix(field, name+"=") {
			return true
		}
	}
	return false
}

func artifactTitle(body string) string {
	for _, line := range strings.Split(body, "\n") {
		if title, ok := strings.CutPrefix(line, "# "); ok {
			return strings.TrimSpace(title)
		}
	}
	return ""
}

func quoted(s string) string {
	if s == "" {
		return ""
	}
	return fmt.Sprintf(" (%q)", s)
}

func shortHash(h string) string {
	if len(h) > 12 {
		return h[:12]
	}
	return h
}

func askGuard(stdout io.Writer, reason string) error {
	response := map[string]any{"hookSpecificOutput": map[string]any{"hookEventName": "PreToolUse", "permissionDecision": "ask", "permissionDecisionReason": reason}}
	return json.NewEncoder(stdout).Encode(response)
}

func denyGuard(stdout io.Writer, reason string) error {
	response := map[string]any{"hookSpecificOutput": map[string]any{"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason}}
	return json.NewEncoder(stdout).Encode(response)
}
