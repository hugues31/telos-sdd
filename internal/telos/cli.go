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
	"github.com/hugues31/telos-sdd/internal/glob"
	"github.com/hugues31/telos-sdd/internal/kernel"
)

const usage = `Telos — certified-state development: every accepted state is verified

Usage:
  telos init [--agent codex|claude|all] [--ci github] [--json]
  telos status [--json]
  telos verify [--json]
  telos doctor [--json]
  telos change start --category behavior_change|behavior_preserving --title <t> [--json]
  telos change show|diff|review [--json]           (inside the candidate)
  telos change approve --digest <oid> [--json]     (inside the candidate)
  telos change ready|promote|rebase [--json]       (inside the candidate)
  telos change abort CHG-NNN [--json]              (from the certified root)
  telos salvage [--into CHG-NNN] [--title <t>] [--json]
  telos restore [--json]
  telos evidence red|green|adopt --req REQ-NNN [--json]
  telos findings list|add|confirm|resolve [...] [--json]
  telos index rebuild|status [--json]
  telos search <query> [--json]
  telos show <ID> [--json]
  telos related <ID> [--depth N] [--all-edges] [--json]
  telos impact <ID> [--json]
  telos explain <symbol> [--json]
  telos context [--budget N] [--focus IDs] [query...] [--json]
  telos guard
  telos version [--json]

Certification policy lives in policies/*.cue over an embedded kernel floor.
The web view arrives with M8 of the v0.6 rewrite; see docs/design-v2.md.`

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
			switch {
			case st.Context == "candidate" && st.Change != nil:
				human = fmt.Sprintf("Candidate %s (%s, %s).", st.Change.ID, st.Change.Category, st.Change.Status)
				if st.Change.BaseStale {
					human += " The base moved: rebase before promotion."
				}
			case st.State == kernel.StateUninitialized:
				next = []string{"init"}
			case st.State == kernel.StateCorrupted:
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
	case "change":
		execution, err := runChange(repo, version, args[1:], commandStdout, stderr)
		return finish(execution, err)
	case "salvage":
		f := flags("salvage", stderr)
		into := f.String("into", "", "route the diff into an open CHG-NNN")
		title := f.String("title", "", "title for the new change")
		if err := f.Parse(args[1:]); err != nil {
			return finish(commandExecution{}, err)
		}
		result, err := kernel.Salvage(repo, strings.ToUpper(*into), *title)
		if err != nil {
			return finish(commandExecution{}, err)
		}
		human := fmt.Sprintf("Captured %d path(s) into %s.\nYour work moved to %s; the certified worktree was restored.", len(result.Paths), result.Change, result.Worktree)
		if len(result.SpecTouched) > 0 {
			human += "\nNote: the diff touches spec/ — move those edits into contract.delta.md before review."
		}
		return finish(commandExecution{Command: "salvage", Result: result, Next: []string{"change review"}, Human: human}, nil)
	case "restore":
		paths, err := kernel.Restore(repo)
		if err != nil {
			return finish(commandExecution{}, err)
		}
		human := fmt.Sprintf("Restored the certified state; %d path(s) discarded.", len(paths))
		return finish(commandExecution{Command: "restore", Result: map[string]any{"paths": paths}, Human: human}, nil)
	case "evidence":
		execution, err := runEvidence(repo, args[1:], commandStdout, stderr)
		return finish(execution, err)
	case "findings":
		execution, err := runFindings(repo, args[1:], stderr)
		return finish(execution, err)
	case "index":
		execution, err := runIndex(root, args[1:])
		return finish(execution, err)
	case "search":
		execution, err := runSearch(root, args[1:])
		return finish(execution, err)
	case "show":
		execution, err := runShow(root, args[1:])
		return finish(execution, err)
	case "related":
		execution, err := runRelated(root, args[1:], stderr)
		return finish(execution, err)
	case "impact":
		execution, err := runImpact(root, args[1:])
		return finish(execution, err)
	case "explain":
		execution, err := runExplain(root, args[1:])
		return finish(execution, err)
	case "context":
		execution, err := runContext(repo, root, args[1:], stderr)
		return finish(execution, err)
	default:
		return finish(commandExecution{}, coded.New("TELOS_INPUT_INVALID", fmt.Sprintf("unknown command %q; run `telos help`", args[0])))
	}
}

func runChange(repo *gitx.Repo, version string, args []string, stdout, stderr io.Writer) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "change requires a verb: start, show, diff, review, approve, ready, promote, or abort")
	}
	switch args[0] {
	case "start":
		f := flags("change start", stderr)
		category := f.String("category", "", "behavior_change or behavior_preserving")
		title := f.String("title", "", "short human title")
		if err := f.Parse(args[1:]); err != nil {
			return commandExecution{}, err
		}
		if *category == "" {
			return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "change start requires --category")
		}
		doc, worktree, err := kernel.StartChange(repo, *category, *title)
		if err != nil {
			return commandExecution{}, err
		}
		result := map[string]any{"id": doc.ID, "worktree": worktree, "branch": doc.Branch, "base": doc.Base, "category": doc.Category}
		human := fmt.Sprintf("Started %s in %s.\nDevelop there; describe contract changes in changes/%s/contract.delta.md.", doc.ID, worktree, doc.ID)
		return commandExecution{Command: "change.start", Result: result, Next: []string{"change review"}, Human: human}, nil
	case "show":
		doc, err := kernel.LoadChange(repo)
		if err != nil {
			return commandExecution{}, err
		}
		paths, _ := repo.DiffNames(doc.Base, "HEAD")
		return commandExecution{Command: "change.show", Result: map[string]any{"change": doc, "changed_paths": paths}}, nil
	case "diff":
		doc, err := kernel.LoadChange(repo)
		if err != nil {
			return commandExecution{}, err
		}
		patch, err := repo.DiffPatch(doc.Base, "HEAD")
		if err != nil {
			return commandExecution{}, err
		}
		return commandExecution{Command: "change.diff", Result: map[string]any{"id": doc.ID, "patch": patch}}, nil
	case "review":
		doc, bundle, err := kernel.ReviewChange(repo)
		if err != nil {
			return commandExecution{}, err
		}
		human := fmt.Sprintf("Review %s recorded for %s (%s). Present the exact content to the human before approval.", shortDigest(bundle.Digest), doc.ID, bundle.Kind)
		return commandExecution{Command: "change.review", Result: bundle, Next: []string{"change approve"}, Human: human}, nil
	case "approve":
		f := flags("change approve", stderr)
		digest := f.String("digest", "", "the reviewed digest")
		if err := f.Parse(args[1:]); err != nil {
			return commandExecution{}, err
		}
		doc, err := kernel.ApproveChange(repo, *digest)
		if err != nil {
			return commandExecution{}, err
		}
		human := fmt.Sprintf("%s approved (%s bound to %s).", doc.ID, doc.Approvals[len(doc.Approvals)-1].Kind, shortDigest(*digest))
		return commandExecution{Command: "change.approve", Result: map[string]any{"change": doc}, Human: human}, nil
	case "rebase":
		cfg, err := kernel.ReadConfig(repo.WorkDir)
		if err != nil {
			return commandExecution{}, err
		}
		report, err := kernel.RebaseChange(repo, cfg)
		if err != nil {
			return commandExecution{}, err
		}
		human := fmt.Sprintf("%s rebased onto %s: %d evidence record(s) survived, %d invalidated.", report.ID, report.NewBase[:12], len(report.EvidenceKept), len(report.EvidenceInvalidated))
		if !report.ApprovalsKept {
			human += "\nThe contract context changed: review and approve again."
		}
		return commandExecution{Command: "change.rebase", Result: report, Next: []string{"change ready"}, Human: human}, nil
	case "ready":
		cfg, err := kernel.ReadConfig(repo.WorkDir)
		if err != nil {
			return commandExecution{}, err
		}
		report, err := kernel.ReadyChange(repo, cfg, stdout)
		if err != nil {
			return commandExecution{}, err
		}
		human := fmt.Sprintf("%s is ready: every certification gate passes (%d evidence record(s), %d requirement(s)).", report.ID, len(report.Evidence), len(report.Requirements))
		return commandExecution{Command: "change.ready", Result: report, Next: []string{"change promote"}, Human: human}, nil
	case "promote":
		cfg, err := kernel.ReadConfig(repo.WorkDir)
		if err != nil {
			return commandExecution{}, err
		}
		result, err := kernel.PromoteChange(repo, cfg, version, stdout)
		if err != nil {
			return commandExecution{}, err
		}
		human := fmt.Sprintf("%s promoted: %s is now the certified state of %s.", result.ID, result.Commit[:12], result.Branch)
		if result.Cleaned {
			human += "\nThe candidate worktree was removed; return to " + result.Root + "."
		}
		return commandExecution{Command: "change.promote", Result: result, Next: []string{"status"}, Human: human}, nil
	case "abort":
		if len(args) < 2 {
			return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "change abort takes the CHG-NNN id, run from the certified root")
		}
		if ctx, err := kernel.ChangeContext(repo); err != nil {
			return commandExecution{}, err
		} else if ctx != "" {
			return commandExecution{}, coded.New("TELOS_ROOT_REQUIRED", "run change abort from the certified root, not inside the candidate being removed")
		}
		id := strings.ToUpper(args[1])
		if err := kernel.AbortChange(repo, id); err != nil {
			return commandExecution{}, err
		}
		return commandExecution{Command: "change.abort", Result: map[string]any{"id": id}, Human: id + " aborted: worktree and branch removed."}, nil
	default:
		return commandExecution{}, coded.New("TELOS_INPUT_INVALID", fmt.Sprintf("unknown change verb %q", args[0]))
	}
}

func shortDigest(d string) string {
	if len(d) > 12 {
		return d[:12]
	}
	return d
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
	case st.Context == "candidate":
		certErr = nil // candidate worktrees are legitimately in flight
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

// candidateProtected lists the paths an agent may not edit directly even
// inside its own candidate worktree: contract semantics go through the delta,
// evidence and the change record through the broker, and provider assets stay
// kernel-owned. Everything else in a candidate is freely editable.
var candidateProtected = []string{
	"spec/**", "telos.toml", "policies/**",
	"changes/*/change.json", "changes/*/evidence/**", "changes/*/findings.json",
	".claude/**", ".codex/**", ".agents/**", "CLAUDE.md", "AGENTS.md",
}

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
	inCandidate := false
	if repo, err := gitx.Open(root); err == nil {
		if id, err := kernel.ChangeContext(repo); err == nil && id != "" {
			inCandidate = true
		}
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
		if inCandidate {
			return nil // candidates are working space: builds and tests run freely
		}
		return denyGuard(stdout, "Telos strict mode permits shell execution only through the Telos CLI broker.")
	}
	if strings.EqualFold(toolName, "Edit") || strings.EqualFold(toolName, "Write") {
		if inCandidate {
			var toolInput map[string]any
			_ = json.Unmarshal(raw, &toolInput)
			path, _ := toolInput["file_path"].(string)
			rel := relToRoot(root, path)
			if rel == "" {
				return nil // outside the project: not our concern
			}
			if glob.MatchAny(candidateProtected, rel) {
				return denyGuard(stdout, "Telos: "+rel+" is protected even in a candidate — contract changes go through contract.delta.md, evidence and the change record through the broker.")
			}
			return nil
		}
		return denyGuard(stdout, "Telos strict mode denies direct writes in the certified worktree; work in a Change candidate (telos change start).")
	}
	if strings.EqualFold(toolName, "apply_patch") {
		if inCandidate {
			return denyGuard(stdout, "Telos: use per-file edits in the candidate; apply_patch cannot be screened against protected paths.")
		}
		return denyGuard(stdout, "Telos strict mode denies direct writes in the certified worktree; work in a Change candidate (telos change start).")
	}
	return nil
}

// relToRoot converts a tool-provided path to a slash-separated repo-relative
// path, or "" when it falls outside the project.
func relToRoot(root, path string) string {
	if path == "" {
		return ""
	}
	if !filepath.IsAbs(path) {
		path = filepath.Join(root, path)
	}
	rel, err := filepath.Rel(root, path)
	if err != nil || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return ""
	}
	return filepath.ToSlash(rel)
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
// human-gated operations: re-init (destructive reset), change approve
// (digest-bound — denied outright when stale so the human is only prompted
// when approval can succeed), and change abort (destructive).
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
	case fields[1] == "init":
		return askGuard(stdout, "Telos human gate — re-initialize Telos in an already-initialized project: this seals a NEW genesis certificate adopting the current tree as-is, outside any verified transition. Approve only if you deliberately want a destructive reset of the certified state.")
	case fields[1] == "restore":
		return askGuard(stdout, "Telos human gate — restore the certified state: the out-of-band diff is DISCARDED. Approve only if that work should be lost; `telos salvage` preserves it as a Change instead.")
	case fields[1] == "change" && verb == "approve":
		return gateChangeApprove(root, stdout, fields)
	case fields[1] == "change" && verb == "abort":
		subject := "the named change"
		if len(fields) > 3 {
			subject = fields[3]
		}
		return askGuard(stdout, "Telos human gate — abort "+subject+": its candidate worktree and branch are removed, discarding unpromoted work. Approve only if that work should be lost.")
	case fields[1] == "evidence" && verb == "adopt":
		subject := flagValue(fields, "--req")
		if subject == "" {
			subject = "the cited requirement"
		}
		return askGuard(stdout, "Telos human gate — adopt existing behavior as proof: the test for "+subject+" is expected to pass immediately, so it will never be witnessed failing. Approve only if the requirement documents behavior the code already has; new behavior must enter through a witnessed failing test.")
	case fields[1] == "findings" && (verb == "confirm" || verb == "resolve"):
		subject := "the named finding"
		if len(fields) > 3 {
			subject = fields[3]
		}
		if verb == "confirm" {
			return askGuard(stdout, "Telos human gate — confirm "+subject+" at its proposed severity: a confirmed blocking finding forbids certification until resolved. This is the human decision the critic cannot make.")
		}
		return askGuard(stdout, "Telos human gate — resolve "+subject+": resolving is the human judgment that closes it (real, not_an_issue, or duplicate). A blocking finding resolved here stops gating certification.")
	}
	return nil
}

// gateChangeApprove denies stale approvals outright and asks otherwise,
// naming the digest and what it binds. It reads the recorded review without
// recomputation (the command itself recomputes on the exact content).
func gateChangeApprove(root string, stdout io.Writer, fields []string) error {
	repo, err := gitx.Open(root)
	if err != nil {
		return denyGuard(stdout, "Telos human gate: cannot open the candidate repository.")
	}
	doc, err := kernel.LoadChange(repo)
	if err != nil {
		return denyGuard(stdout, "Telos human gate: change approve runs inside a candidate with a recorded review; run telos change review first.")
	}
	digest := flagValue(fields, "--digest")
	if doc.Review == nil || digest == "" || digest != doc.Review.Digest {
		return denyGuard(stdout, "Telos human gate: the review digest is missing or stale; run telos change review and present the returned content to the user before approving.")
	}
	subject := doc.ID + " (" + doc.Category + ")"
	claim := "this exact contract delta was presented to you and is exactly the intended behavior"
	if doc.Review.Kind == "preserving_claim" {
		claim = "this change was presented to you and preserves all certified behavior (refactor or hardening; a bug fix must strengthen the contract instead)"
	}
	prompt := "Telos human gate — approve " + subject + " with digest " + shortDigest(digest) + ". Approve only if " + claim + "."
	if doc.Privileged {
		prompt = "PRIVILEGED " + prompt + " This change touches certification policy content (telos.toml or policies/): review it with elevated scrutiny."
	}
	return askGuard(stdout, prompt)
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

func askGuard(stdout io.Writer, reason string) error {
	response := map[string]any{"hookSpecificOutput": map[string]any{"hookEventName": "PreToolUse", "permissionDecision": "ask", "permissionDecisionReason": reason}}
	return json.NewEncoder(stdout).Encode(response)
}

func denyGuard(stdout io.Writer, reason string) error {
	response := map[string]any{"hookSpecificOutput": map[string]any{"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason}}
	return json.NewEncoder(stdout).Encode(response)
}
