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

const usage = `Telos SDD — the spec lives in the repo; code follows the approved spec

Usage:
  telos init [--agent codex|claude|all] [--ci github] [--json]
  telos doctor [--json]
  telos status [--json]
  telos spec put --file spec/<name>.md [--delete] [--json] < content.md
  telos spec review [--json]
  telos spec approve --review <digest> [--json]
  telos apply --rule RULE-NNN [--rule ...] [--expect-pass] [--json] < patch.diff
  telos verify [--json]
  telos trace [RULE-NNN] [--json]
  telos view [--out <path>] [--open] [--json]
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
	switch args[0] {
	case "doctor":
		return finish(commandExecution{}, runDoctor(root, commandStdout))
	case "status":
		result, next, err := runStatus(root)
		human := ""
		if err == nil {
			human = fmt.Sprintf("Phase: %s.", result["phase"])
		}
		return finish(commandExecution{Command: "status", Result: result, Next: next, Human: human}, err)
	case "spec":
		execution, err := runSpec(root, args[1:], stdin, stderr)
		return finish(execution, err)
	case "apply":
		f := flags("apply", stderr)
		var rules stringList
		f.Var(&rules, "rule", "RULE-NNN reference (repeatable)")
		expectPass := f.Bool("expect-pass", false, "adopt existing behavior: the suite must pass with the documentation test in place (human-gated)")
		if err := f.Parse(args[1:]); err != nil {
			return finish(commandExecution{}, err)
		}
		patch, err := io.ReadAll(stdin)
		if err != nil {
			return finish(commandExecution{}, err)
		}
		result, err := runApply(root, rules, patch, *expectPass, stderr)
		human := ""
		if err == nil {
			human = fmt.Sprintf("Applied patch for %s through Telos.", strings.Join(rules, ", "))
			switch result["suite"] {
			case "red":
				human = fmt.Sprintf("Red witnessed for %s: the failing tests are sealed; only the implementation may turn them green.", strings.Join(rules, ", "))
			case "green":
				if proven, ok := result["proven"].([]string); ok && len(proven) > 0 {
					human = fmt.Sprintf("Suite green witnessed through Telos; proven: %s.", strings.Join(proven, ", "))
				}
			}
		}
		return finish(commandExecution{Command: "apply", Result: result, Next: []string{"verify"}, Human: human}, err)
	case "verify":
		result, err := runVerify(root, commandStdout, stderr)
		human := ""
		if err == nil {
			human = fmt.Sprintf("Verified: %d rule(s) implemented, spec and code in sync.", result["rules"])
		}
		return finish(commandExecution{Command: "verify", Result: result, Human: human}, err)
	case "trace":
		id := ""
		if len(args) > 1 {
			id = args[1]
		}
		result, err := runTrace(root, id)
		return finish(commandExecution{Command: "trace", Result: result}, err)
	case "view":
		f := flags("view", stderr)
		out := f.String("out", "", "output HTML path (default: system temp; inside the repo it must be git-ignored)")
		open := f.Bool("open", false, "open the page in the default browser")
		if err := f.Parse(args[1:]); err != nil {
			return finish(commandExecution{}, err)
		}
		result, err := runView(root, version, *out, *open)
		human := ""
		if err == nil {
			human = fmt.Sprintf("Spec view written to %s.", result["path"])
		}
		return finish(commandExecution{Command: "view", Result: result, Human: human}, err)
	default:
		return finish(commandExecution{}, fmt.Errorf("unknown command %q; run `telos help`", args[0]))
	}
}

func runSpec(root string, args []string, stdin io.Reader, stderr io.Writer) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, coded("TELOS_INPUT_REQUIRED", "spec requires a verb: put, review, or approve")
	}
	switch args[0] {
	case "put":
		f := flags("spec put", stderr)
		file := f.String("file", "", "spec file path under spec/")
		del := f.Bool("delete", false, "delete the spec file")
		if err := f.Parse(args[1:]); err != nil {
			return commandExecution{}, err
		}
		var content []byte
		if !*del {
			var err error
			if content, err = io.ReadAll(stdin); err != nil {
				return commandExecution{}, err
			}
		}
		result, err := specPut(root, *file, content, *del)
		human := ""
		if err == nil {
			human = fmt.Sprintf("Wrote %s through Telos.", result["path"])
			if *del {
				human = fmt.Sprintf("Deleted %s through Telos.", result["path"])
			}
		}
		return commandExecution{Command: "spec.put", Result: result, Next: []string{"spec review"}, Human: human}, err
	case "review":
		result, err := specReview(root)
		human := ""
		if err == nil {
			human = fmt.Sprintf("Spec review digest %s. Present the returned content to the user before approval.", shortHash(fmt.Sprint(result["digest"])))
		}
		return commandExecution{Command: "spec.review", Result: result, Next: []string{"spec approve"}, Human: human}, err
	case "approve":
		f := flags("spec approve", stderr)
		review := f.String("review", "", "review digest")
		if err := f.Parse(args[1:]); err != nil {
			return commandExecution{}, err
		}
		result, err := specApprove(root, *review)
		human := ""
		if err == nil {
			human = "Spec approved; the approved root is the new implementation target."
		}
		return commandExecution{Command: "spec.approve", Result: result, Next: []string{"apply", "verify"}, Human: human}, err
	default:
		return commandExecution{}, coded("TELOS_INPUT_INVALID", fmt.Sprintf("unknown spec verb %q", args[0]))
	}
}

type stringList []string

func (s *stringList) String() string { return strings.Join(*s, ",") }
func (s *stringList) Set(v string) error {
	*s = append(*s, strings.ToUpper(strings.TrimSpace(v)))
	return nil
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
	fmt.Fprintf(stdout, "Initialized Telos SDD in %s for %s.\nThe spec lives in spec/; tell your coding agent what you want to build.\n", cwd, *agent)
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
	type check struct {
		Name string
		Err  error
	}
	cfg, cfgErr := readConfig(root)
	_, stateErr := loadState(root)
	checks := []check{
		{"Telos config", cfgErr},
		{"Telos state", stateErr},
		{"Product spec", fileExists(filepath.Join(root, filepath.FromSlash(productFile)))},
		{"Git", commandExists("git")},
	}
	for _, agent := range cfg.Agents {
		switch agent {
		case "codex":
			checks = append(checks, check{"Codex Skill", fileExists(filepath.Join(root, ".agents", "skills", "telos", "SKILL.md"))})
		case "claude":
			checks = append(checks, check{"Claude Skill", fileExists(filepath.Join(root, ".claude", "skills", "telos", "SKILL.md"))})
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

func fileExists(path string) error    { _, err := os.Stat(path); return err }
func commandExists(name string) error { _, err := exec.LookPath(name); return err }

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
// human-gated operations: spec approve (the single workflow gate), re-init
// (the administrative re-baseline escape hatch), and apply on a clean project
// (the refactor claim) or with --expect-pass (the adoption claim). These never
// pass silently — the human decision happens
// at the provider permission prompt, not on the orchestrator's word. An
// approve whose digest no longer matches the recorded review is denied
// outright so the user is only prompted when it can succeed.
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
	case fields[1] == "spec" && verb == "approve":
		return gateSpecApprove(root, stdout, fields)
	case fields[1] == "init":
		return askGuard(stdout, "Telos human gate — re-initialize Telos in an already-initialized project: this re-baselines the declared spec and code roots, adopting the current tree as-is. Approve only if you deliberately want to reset the declared state.")
	case fields[1] == "apply":
		return gateApply(root, stdout, command, fields)
	}
	return nil
}

// gateApply lets spec-driven patches through silently: while approved rules
// still lack proof, the human decision already happened at spec approval. Two
// applies carry a claim only the human can accept and are prompted: an apply
// on a clean project — a code change no spec diff motivates, claiming behavior
// preservation (refactor, test hardening); a reported bug never qualifies,
// since it is evidence the spec was too weak — and an apply with --expect-pass,
// which claims a rule is already satisfied by the current code, so its test
// will never be witnessed failing. On any other phase the guard stays silent
// and the command itself fails with the precise error code.
func gateApply(root string, stdout io.Writer, command string, fields []string) error {
	result, _, err := runStatus(root)
	if err != nil {
		return nil
	}
	subject := strings.Join(flagValues(fields, "--rule"), ", ")
	if subject == "" {
		subject = "unspecified rules"
	}
	if hasFlag(fields, "--expect-pass") {
		if result["phase"] != "implementing" {
			return nil
		}
		return askGuard(stdout, fmt.Sprintf("Telos human gate — adopt existing behavior as proof: the test for %s is expected to pass immediately, so it will never be witnessed failing. Approve only if the rule documents behavior the code already has; new behavior must enter through a witnessed failing test.", subject))
	}
	if result["phase"] != "clean" {
		return nil
	}
	if files := patchFiles(command); len(files) > 0 {
		subject += " touching " + strings.Join(files, ", ")
	}
	return askGuard(stdout, fmt.Sprintf("Telos human gate — code change without a spec change: the project is clean, so this patch for %s claims to preserve behavior (refactor or test hardening). Approve only if no behavior changes; a bug fix must strengthen the spec first.", subject))
}

func hasFlag(fields []string, name string) bool {
	for _, field := range fields {
		if field == name {
			return true
		}
	}
	return false
}

// patchFiles lists the paths targeted by the unified diff carried in the
// command's heredoc body, so the permission prompt names what the patch
// touches.
func patchFiles(command string) []string {
	var out []string
	seen := map[string]bool{}
	for _, line := range strings.Split(strings.ReplaceAll(command, "\r\n", "\n"), "\n") {
		rest, ok := strings.CutPrefix(strings.TrimSpace(line), "diff --git a/")
		if !ok {
			continue
		}
		if i := strings.LastIndex(rest, " b/"); i >= 0 {
			rest = rest[:i]
		}
		if rest != "" && !seen[rest] {
			seen[rest] = true
			out = append(out, rest)
		}
	}
	return out
}

func gateSpecApprove(root string, stdout io.Writer, fields []string) error {
	digest := flagValue(fields, "--review")
	st, err := loadState(root)
	if err != nil {
		return denyGuard(stdout, "Telos human gate: no recorded state; run telos spec review before approving.")
	}
	_, specFiles, invErr := inventories(root)
	if invErr != nil || st.Review == "" || digest == "" || digest != st.Review || rootHashMap(specFiles) != st.Review {
		return denyGuard(stdout, "Telos human gate: the spec review digest is missing or stale; run telos spec review and present the returned content to the user before approving.")
	}
	changed := changedPaths(st.Spec.Files, specFiles)
	return askGuard(stdout, fmt.Sprintf("Telos human gate — approve the spec diff %s covering %s. Approve only if this exact spec content was presented to you and is exactly the intended behavior.", shortHash(digest), strings.Join(changed, ", ")))
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

func flagValues(fields []string, name string) []string {
	var out []string
	for i, field := range fields {
		if field == name && i+1 < len(fields) {
			out = append(out, strings.Trim(fields[i+1], `"'`))
		}
		if value, ok := strings.CutPrefix(field, name+"="); ok {
			out = append(out, strings.Trim(value, `"'`))
		}
	}
	return out
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
