package telos

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"
)

func runCommand(root string, args []string, stdin io.Reader, stdout, stderr io.Writer) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, errors.New("command is required")
	}
	switch args[0] {
	case "flow":
		return runFlow(root, args[1:], stdin, stderr)
	case "inspect":
		if len(args) != 1 {
			return commandExecution{}, errors.New("inspect takes no arguments")
		}
		execution, err := inspectProject(root)
		return execution, err
	case "artifact":
		return runArtifact(root, args[1:], stdin, stderr)
	case "test-plan":
		return runTestPlan(root, args[1:], stdin, stderr)
	case "contract":
		return runContract(root, args[1:], stderr)
	case "repair":
		return runRepair(root, args[1:], stderr)
	case "intent":
		return runIntent(root, args[1:], stderr)
	case "spec":
		return runSpec(root, args[1:], stderr)
	case "change":
		return runChange(root, args[1:], stdin, stdout, stderr)
	case "verify":
		return runVerify(root, args[1:], stdout, stderr)
	}
	return commandExecution{}, fmt.Errorf("unknown command %q", args[0])
}

type repeated []string

func (values *repeated) String() string { return strings.Join(*values, ",") }
func (values *repeated) Set(value string) error {
	*values = append(*values, value)
	return nil
}

func readInput(stdin io.Reader, fallback string) (string, error) {
	if strings.TrimSpace(fallback) != "" {
		return strings.TrimSpace(fallback), nil
	}
	data, err := io.ReadAll(stdin)
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(data)), nil
}

func runFlow(root string, args []string, stdin io.Reader, stderr io.Writer) (commandExecution, error) {
	if len(args) == 0 || args[0] != "start" {
		return commandExecution{}, errors.New("usage: telos flow start [--request text] [--brainstorm none|choose|recommend|random|progressive]")
	}
	f := flags("flow start", stderr)
	request := f.String("request", "", "development request")
	brainstorm := f.String("brainstorm", "recommend", "brainstorm mode or none")
	if err := f.Parse(args[1:]); err != nil {
		return commandExecution{}, err
	}
	input, err := readInput(stdin, *request)
	if err != nil {
		return commandExecution{}, err
	}
	flow, err := startFlow(root, input, *brainstorm)
	if err != nil {
		return commandExecution{}, err
	}
	next := nextActions(flow)
	return commandExecution{Command: "flow.start", Result: flow, Next: next, Human: fmt.Sprintf("Flow %s started in phase %s.", flow.ID, flow.Phase)}, nil
}

func inspectProject(root string) (commandExecution, error) {
	auditResults, err := audit(root)
	if err != nil {
		return commandExecution{}, err
	}
	for _, result := range auditResults {
		if result.Status != "ok" {
			return commandExecution{}, codedPaths("TELOS_INTEGRITY_ARTIFACT", fmt.Sprintf("%s: %s (%s)", result.Status, result.Path, result.Detail), []string{result.Path})
		}
	}
	lock, err := loadLock(root)
	if err != nil {
		return commandExecution{}, err
	}
	result := map[string]any{"root_hash": empty(lock.RootHash, "unsealed"), "phase": "idle", "flow": nil}
	flow, err := activeFlow(root)
	if errors.Is(err, os.ErrNotExist) {
		return commandExecution{Command: "inspect", Result: result, Next: []string{"flow.start"}, Human: "No active Telos flow."}, nil
	}
	if err != nil {
		return commandExecution{}, err
	}
	if err := auditFlowDrafts(root, flow); err != nil {
		return commandExecution{}, err
	}
	result["phase"] = flow.Phase
	result["flow"] = flow
	if flow.Change != "" {
		var change Change
		if err := readJSON(filepath.Join(root, ".telos", "changes", strings.ToLower(flow.Change)+".json"), &change); err == nil {
			result["change"] = change
		}
	}
	return commandExecution{Command: "inspect", Result: result, Next: nextActions(flow), Human: fmt.Sprintf("Flow %s: %s.", flow.ID, flow.Phase)}, nil
}

func nextActions(flow Flow) []string {
	switch flow.Phase {
	case "brainstorming":
		return []string{"artifact.put", "intent.new"}
	case "intent_draft":
		return []string{"artifact.put", "intent.review"}
	case "intent_review":
		return []string{"intent.seal"}
	case "contract_draft":
		return []string{"spec.new", "artifact.put", "test-plan.put", "contract.review"}
	case "contract_review":
		return []string{"contract.seal"}
	case "ready_to_implement":
		return []string{"change.begin"}
	case "implementing":
		return []string{"change.apply", "verify.check-only", "change.complete"}
	default:
		return nil
	}
}

func runArtifact(root string, args []string, stdin io.Reader, stderr io.Writer) (commandExecution, error) {
	if len(args) == 0 || (args[0] != "put" && args[0] != "revise") {
		return commandExecution{}, errors.New("usage: telos artifact put|revise --id <INT|SPC-id>")
	}
	f := flags("artifact "+args[0], stderr)
	id := f.String("id", "", "artifact id")
	body := f.String("body", "", "Markdown body; stdin when omitted")
	reason := f.String("reason", "", "revision and active-change abort reason")
	if err := f.Parse(args[1:]); err != nil {
		return commandExecution{}, err
	}
	if *id == "" {
		return commandExecution{}, coded("TELOS_INPUT_REQUIRED", "--id is required")
	}
	if args[0] == "revise" {
		flow, newID, path, err := reviseArtifact(root, *id, *reason)
		return commandExecution{Command: "artifact.revise", Result: map[string]any{"flow": flow, "id": newID, "path": path, "supersedes": *id}, Next: nextActions(flow), Human: fmt.Sprintf("Created revision %s through Telos.", newID)}, err
	}
	input, err := readInput(stdin, *body)
	if err != nil {
		return commandExecution{}, err
	}
	path, err := putArtifact(root, *id, input)
	if err != nil {
		return commandExecution{}, err
	}
	return commandExecution{Command: "artifact.put", Result: map[string]any{"id": *id, "path": path}, Next: []string{"inspect"}, Human: fmt.Sprintf("Updated %s through Telos.", *id)}, nil
}

func runTestPlan(root string, args []string, stdin io.Reader, stderr io.Writer) (commandExecution, error) {
	if len(args) == 0 || args[0] != "put" {
		return commandExecution{}, errors.New("usage: telos test-plan put --spec <id>")
	}
	f := flags("test-plan put", stderr)
	spec := f.String("spec", "", "spec id")
	if err := f.Parse(args[1:]); err != nil {
		return commandExecution{}, err
	}
	if *spec == "" {
		return commandExecution{}, coded("TELOS_INPUT_REQUIRED", "--spec is required")
	}
	data, err := io.ReadAll(stdin)
	if err != nil {
		return commandExecution{}, err
	}
	path, err := putTestPlan(root, *spec, data)
	if err != nil {
		return commandExecution{}, err
	}
	return commandExecution{Command: "test-plan.put", Result: map[string]any{"spec": *spec, "path": path}, Next: []string{"contract.review"}, Human: fmt.Sprintf("Updated the test plan for %s through Telos.", *spec)}, nil
}

func runIntent(root string, args []string, stderr io.Writer) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, errors.New("usage: telos intent new|review|seal --flow <id>")
	}
	f := flags("intent "+args[0], stderr)
	flowID := f.String("flow", "", "flow id")
	title := f.String("title", "", "intent title")
	review := f.String("review", "", "approved review digest")
	if err := f.Parse(args[1:]); err != nil {
		return commandExecution{}, err
	}
	if *flowID == "" {
		return commandExecution{}, coded("TELOS_INPUT_REQUIRED", "--flow is required")
	}
	switch args[0] {
	case "new":
		flow, path, err := attachIntent(root, *flowID, *title)
		return commandExecution{Command: "intent.new", Result: map[string]any{"flow": flow, "path": path}, Next: nextActions(flow), Human: fmt.Sprintf("Intent created for flow %s.", flow.ID)}, err
	case "review":
		flow, digest, body, err := reviewIntent(root, *flowID)
		return commandExecution{Command: "intent.review", Result: map[string]any{"flow": flow.ID, "digest": digest, "content": body}, Next: []string{"intent.seal"}, Human: body}, err
	case "seal":
		flow, err := sealReviewedIntent(root, *flowID, *review)
		return commandExecution{Command: "intent.seal", Result: flow, Next: nextActions(flow), Human: "Approved intent sealed."}, err
	default:
		return commandExecution{}, fmt.Errorf("unknown intent command %q", args[0])
	}
}

func runSpec(root string, args []string, stderr io.Writer) (commandExecution, error) {
	if len(args) == 0 || args[0] != "new" {
		return commandExecution{}, errors.New("usage: telos spec new --flow <id>")
	}
	f := flags("spec new", stderr)
	flowID := f.String("flow", "", "flow id")
	title := f.String("title", "", "spec title")
	if err := f.Parse(args[1:]); err != nil {
		return commandExecution{}, err
	}
	if *flowID == "" {
		return commandExecution{}, coded("TELOS_INPUT_REQUIRED", "--flow is required")
	}
	flow, path, err := attachSpec(root, *flowID, *title)
	return commandExecution{Command: "spec.new", Result: map[string]any{"flow": flow, "path": path}, Next: nextActions(flow), Human: fmt.Sprintf("Spec added to flow %s.", flow.ID)}, err
}

func runContract(root string, args []string, stderr io.Writer) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, errors.New("usage: telos contract validate|review|seal --flow <id>")
	}
	f := flags("contract "+args[0], stderr)
	flowID := f.String("flow", "", "flow id")
	review := f.String("review", "", "approved review digest")
	if err := f.Parse(args[1:]); err != nil {
		return commandExecution{}, err
	}
	if *flowID == "" {
		return commandExecution{}, coded("TELOS_INPUT_REQUIRED", "--flow is required")
	}
	switch args[0] {
	case "validate":
		flow, err := validateContract(root, *flowID)
		return commandExecution{Command: "contract.validate", Result: flow, Next: []string{"contract.review"}, Human: "Contract validation passed."}, err
	case "review":
		flow, digest, summary, err := reviewContract(root, *flowID)
		return commandExecution{Command: "contract.review", Result: map[string]any{"flow": flow.ID, "digest": digest, "content": summary}, Next: []string{"contract.seal"}, Human: summary}, err
	case "seal":
		flow, err := sealReviewedContract(root, *flowID, *review)
		return commandExecution{Command: "contract.seal", Result: flow, Next: nextActions(flow), Human: "Approved contract sealed atomically."}, err
	default:
		return commandExecution{}, fmt.Errorf("unknown contract command %q", args[0])
	}
}

func runChange(root string, args []string, stdin io.Reader, stdout, stderr io.Writer) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, errors.New("usage: telos change begin|apply|complete --flow <id>")
	}
	f := flags("change "+args[0], stderr)
	flowID := f.String("flow", "", "flow id")
	changeID := f.String("change", "", "change id")
	evidence := f.String("evidence", "", "verifier evidence; stdin when omitted")
	reason := f.String("reason", "", "abort reason")
	var rules repeated
	var scenarios repeated
	f.Var(&rules, "rule", "RULE id (repeatable)")
	f.Var(&scenarios, "scenario", "SCN id (repeatable)")
	if err := f.Parse(args[1:]); err != nil {
		return commandExecution{}, err
	}
	switch args[0] {
	case "begin":
		if *flowID == "" {
			return commandExecution{}, coded("TELOS_INPUT_REQUIRED", "--flow is required")
		}
		flow, change, err := beginFlowChange(root, *flowID)
		if err == nil {
			_, err = buildContext(root, change.ID)
			if err != nil {
				_, _ = abortChange(root, change, "context generation failed")
				flow.Change = ""
				flow.Phase = "ready_to_implement"
				_ = saveFlow(root, flow)
			}
		}
		return commandExecution{Command: "change.begin", Result: map[string]any{"flow": flow, "change": change, "context": ".telos/context.md"}, Next: nextActions(flow), Human: fmt.Sprintf("Change %s started and context generated.", change.ID)}, err
	case "apply":
		change, err := resolveChange(root, *flowID, *changeID)
		if err != nil {
			return commandExecution{}, err
		}
		patch, err := io.ReadAll(stdin)
		if err != nil {
			return commandExecution{}, err
		}
		change, mutation, err := applyChangePatch(root, change, patch, rules, scenarios)
		return commandExecution{Command: "change.apply", Result: map[string]any{"change": change, "mutation": mutation}, Next: []string{"verify.check-only"}, Human: fmt.Sprintf("Patch %s applied through Telos.", mutation.ID)}, err
	case "abort":
		change, err := resolveChange(root, *flowID, *changeID)
		if err != nil {
			return commandExecution{}, err
		}
		change, err = abortChange(root, change, *reason)
		if err != nil {
			return commandExecution{}, err
		}
		flow, err := loadFlow(root, change.Flow)
		if err != nil {
			return commandExecution{}, err
		}
		flow.Change = ""
		flow.Phase = "ready_to_implement"
		if err := saveFlow(root, flow); err != nil {
			return commandExecution{}, err
		}
		return commandExecution{Command: "change.abort", Result: map[string]any{"change": change, "flow": flow}, Next: []string{"artifact.revise"}, Human: "Change aborted and declared patches reversed."}, nil
	case "complete":
		change, err := resolveChange(root, *flowID, *changeID)
		if err != nil {
			return commandExecution{}, err
		}
		input, err := readInput(stdin, *evidence)
		if err != nil {
			return commandExecution{}, err
		}
		if input == "" {
			return commandExecution{}, coded("TELOS_INPUT_REQUIRED", "independent verifier evidence is required")
		}
		verification, err := verifyProject(root, stdout, stderr, true)
		if err != nil {
			return commandExecution{}, err
		}
		originalChange := change
		change.Status = "complete"
		change.Completed = time.Now().UTC().Format(time.RFC3339)
		if err := writeJSON(filepath.Join(root, ".telos", "changes", strings.ToLower(change.ID)+".json"), change); err != nil {
			return commandExecution{}, err
		}
		flow, err := loadFlow(root, change.Flow)
		if err != nil {
			return commandExecution{}, err
		}
		originalFlow := flow
		flow.Status = "complete"
		flow.Phase = "complete"
		flow.Verdict = "verified"
		if err := saveFlow(root, flow); err != nil {
			return commandExecution{}, err
		}
		lock, _ := loadLock(root)
		if err := appendEvent(root, "change.completed", change.ID, map[string]any{"evidence": input, "repository_root": change.SourceCurrentRoot}, lock.RootHash); err != nil {
			_ = writeJSON(filepath.Join(root, ".telos", "changes", strings.ToLower(originalChange.ID)+".json"), originalChange)
			_ = saveFlow(root, originalFlow)
			return commandExecution{}, err
		}
		return commandExecution{Command: "change.complete", Result: map[string]any{"change": change, "flow": flow, "verification": verification}, Human: "Change completed with independent verifier evidence."}, nil
	default:
		return commandExecution{}, fmt.Errorf("unknown change command %q", args[0])
	}
}

func resolveChange(root, flowID, changeID string) (Change, error) {
	if changeID == "" && flowID != "" {
		flow, err := loadFlow(root, flowID)
		if err != nil {
			return Change{}, err
		}
		changeID = flow.Change
	}
	if changeID == "" {
		return Change{}, coded("TELOS_INPUT_REQUIRED", "--flow or --change is required")
	}
	var change Change
	err := readJSON(filepath.Join(root, ".telos", "changes", strings.ToLower(changeID)+".json"), &change)
	return change, err
}

func runVerify(root string, args []string, stdout, stderr io.Writer) (commandExecution, error) {
	f := flags("verify", stderr)
	checkOnly := f.Bool("check-only", false, "do not record verification")
	flowID := f.String("flow", "", "flow id")
	if err := f.Parse(args); err != nil {
		return commandExecution{}, err
	}
	if !*checkOnly {
		return commandExecution{}, coded("TELOS_INPUT_REQUIRED", "verify requires --check-only")
	}
	if *flowID != "" {
		flow, err := loadFlow(root, *flowID)
		if err != nil {
			return commandExecution{}, err
		}
		if flow.Phase != "implementing" {
			return commandExecution{}, coded("TELOS_PHASE_INVALID", "flow is not ready for implementation verification")
		}
	}
	result, err := verifyProject(root, stdout, stderr, true)
	return commandExecution{Command: "verify.check-only", Result: result, Next: []string{"change.complete"}, Human: "Independent deterministic verification passed."}, err
}

func runRepair(root string, args []string, stderr io.Writer) (commandExecution, error) {
	f := flags("repair", stderr)
	restore := f.Bool("restore", false, "restore the last declared repository state")
	if err := f.Parse(args); err != nil {
		return commandExecution{}, err
	}
	if !*restore {
		changed, expected, actual, err := auditRepository(root)
		if err != nil {
			return commandExecution{}, err
		}
		artifactResults, auditErr := audit(root)
		if auditErr != nil {
			return commandExecution{}, auditErr
		}
		for _, result := range artifactResults {
			if result.Status != "ok" {
				changed = append(changed, result.Path)
			}
		}
		if flow, flowErr := activeFlow(root); flowErr == nil {
			if draftErr := auditFlowDrafts(root, flow); draftErr != nil {
				changed = append(changed, "active flow artifact")
			}
		}
		return commandExecution{Command: "repair", Result: map[string]any{"paths": changed, "expected_root": expected, "actual_root": actual}, Next: []string{"repair.restore"}, Human: strings.Join(changed, "\n")}, err
	}
	repositoryPaths, err := repairRepository(root)
	if err != nil {
		return commandExecution{}, err
	}
	artifactPaths, err := repairManagedArtifacts(root)
	paths := append(repositoryPaths, artifactPaths...)
	return commandExecution{Command: "repair.restore", Result: map[string]any{"restored": paths}, Next: []string{"inspect"}, Human: fmt.Sprintf("Restored %d paths to the last declared state.", len(paths))}, err
}
