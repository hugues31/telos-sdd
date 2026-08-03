package telos

import (
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
)

const usage = `Telos SDD — intent integrity without a hosted runtime

Usage:
  telos init [--agent codex|claude|all] [--ci github]
  telos doctor
  telos status
  telos brainstorm start [--mode choose|recommend|random|progressive] [--seed n]
  telos intent new [--title text] [--from brainstorm-id]
  telos intent validate <id>
  telos intent seal <id>
  telos spec new --intent <id> [--title text]
  telos spec validate <id>
  telos spec seal <id>
  telos testify --spec <id> [--plan path]
  telos change begin --intent <id> --spec <id> [--spec <id>...]
  telos context --change <id>
  telos verify [--ci]
  telos guard
  telos version`

func Run(args []string, version string, stdin io.Reader, stdout, stderr io.Writer) error {
	if len(args) == 0 {
		fmt.Fprintln(stdout, usage)
		return nil
	}
	if args[0] == "version" {
		fmt.Fprintln(stdout, version)
		return nil
	}
	if args[0] == "help" || args[0] == "--help" || args[0] == "-h" {
		fmt.Fprintln(stdout, usage)
		return nil
	}
	if args[0] == "guard" {
		return runGuard(stdin, stdout)
	}
	cwd, err := os.Getwd()
	if err != nil {
		return err
	}
	if args[0] == "init" {
		return runInit(cwd, args[1:], stdout, stderr)
	}
	root, err := findRoot(cwd)
	if err != nil {
		return err
	}
	switch args[0] {
	case "doctor":
		return runDoctor(root, stdout)
	case "status":
		return runStatus(root, stdout)
	case "brainstorm":
		return runBrainstorm(root, args[1:], stdout, stderr)
	case "intent":
		return runArtifactCommand(root, "intent", args[1:], stdout, stderr)
	case "spec":
		return runArtifactCommand(root, "spec", args[1:], stdout, stderr)
	case "testify":
		return runTestify(root, args[1:], stdout, stderr)
	case "change":
		return runChange(root, args[1:], stdout, stderr)
	case "context":
		return runContext(root, args[1:], stdout, stderr)
	case "verify":
		return runVerify(root, args[1:], stdout, stderr)
	default:
		return fmt.Errorf("unknown command %q\n\n%s", args[0], usage)
	}
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
	fmt.Fprintf(stdout, "Initialized Telos SDD in %s for %s.\nNext: telos brainstorm start --mode recommend\n", cwd, *agent)
	return nil
}

func runDoctor(root string, stdout io.Writer) error {
	checks := []struct {
		Name string
		Err  error
	}{
		{"Telos config", fileExists(filepath.Join(root, ".telos", "config.toml"))},
		{"Git", commandExists("git")},
	}
	cfg, cfgErr := readConfig(root)
	if cfgErr == nil && cfg.Version != configVersion {
		cfgErr = fmt.Errorf("unsupported version %d (CLI supports %d)", cfg.Version, configVersion)
	}
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
			}{"Codex Skills", fileExists(filepath.Join(root, ".agents", "skills", "telos-intent", "SKILL.md"))})
		case "claude":
			checks = append(checks, struct {
				Name string
				Err  error
			}{"Claude Skills", fileExists(filepath.Join(root, ".claude", "skills", "telos-intent", "SKILL.md"))})
		}
	}
	failed := false
	for _, c := range checks {
		status := "ok"
		if c.Err != nil {
			status, failed = c.Err.Error(), true
		}
		fmt.Fprintf(stdout, "%-18s %s\n", c.Name+":", status)
	}
	if failed {
		return errors.New("doctor found configuration errors")
	}
	return nil
}

func runStatus(root string, stdout io.Writer) error {
	results, err := audit(root)
	if err != nil {
		return err
	}
	lock, err := loadLock(root)
	if err != nil {
		return err
	}
	fmt.Fprintf(stdout, "Root hash: %s\n", empty(lock.RootHash, "unsealed"))
	if len(results) == 0 {
		fmt.Fprintln(stdout, "No sealed artifacts yet.")
		return nil
	}
	bad := false
	for _, r := range results {
		fmt.Fprintf(stdout, "%-10s %s", r.Status, r.Path)
		if r.Detail != "" {
			fmt.Fprintf(stdout, " — %s", r.Detail)
		}
		fmt.Fprintln(stdout)
		if r.Status != "ok" {
			bad = true
		}
	}
	if bad {
		return errors.New("project has stale or tampered artifacts")
	}
	return nil
}

func runBrainstorm(root string, args []string, stdout, stderr io.Writer) error {
	if len(args) == 0 || args[0] != "start" {
		return errors.New("usage: telos brainstorm start [--mode ...]")
	}
	f := flags("brainstorm start", stderr)
	mode := f.String("mode", "recommend", "engine selection mode")
	seed := f.Int64("seed", 0, "deterministic random seed")
	if err := f.Parse(args[1:]); err != nil {
		return err
	}
	id, path, err := startBrainstorm(root, *mode, *seed)
	if err != nil {
		return err
	}
	fmt.Fprintf(stdout, "%s\t%s\n", id, path)
	return nil
}

func runArtifactCommand(root, kind string, args []string, stdout, stderr io.Writer) error {
	if len(args) == 0 {
		return fmt.Errorf("usage: telos %s new|validate|seal", kind)
	}
	switch args[0] {
	case "new":
		f := flags(kind+" new", stderr)
		title := f.String("title", "", "artifact title")
		if kind == "intent" {
			from := f.String("from", "", "brainstorm id")
			if err := f.Parse(args[1:]); err != nil {
				return err
			}
			id, path, err := newIntent(root, *title, *from)
			if err != nil {
				return err
			}
			fmt.Fprintf(stdout, "%s\t%s\n", id, path)
			return nil
		}
		intent := f.String("intent", "", "sealed intent id")
		if err := f.Parse(args[1:]); err != nil {
			return err
		}
		if *intent == "" {
			return errors.New("--intent is required")
		}
		id, path, err := newSpec(root, *intent, *title)
		if err != nil {
			return err
		}
		fmt.Fprintf(stdout, "%s\t%s\n", id, path)
		return nil
	case "validate", "seal":
		if len(args) != 2 {
			return fmt.Errorf("usage: telos %s %s <id>", kind, args[0])
		}
		var err error
		if args[0] == "validate" {
			err = validateArtifact(root, kind, args[1])
		} else {
			err = sealArtifact(root, kind, args[1])
		}
		if err != nil {
			return err
		}
		fmt.Fprintf(stdout, "%s %s: ok\n", kind, args[0])
		return nil
	default:
		return fmt.Errorf("unknown %s command %q", kind, args[0])
	}
}

func runTestify(root string, args []string, stdout, stderr io.Writer) error {
	f := flags("testify", stderr)
	spec := f.String("spec", "", "sealed spec id")
	plan := f.String("plan", "", "test plan path")
	if err := f.Parse(args); err != nil {
		return err
	}
	if *spec == "" {
		return errors.New("--spec is required")
	}
	path, err := testify(root, *spec, *plan)
	if err != nil {
		return err
	}
	fmt.Fprintln(stdout, path)
	return nil
}

type repeated []string

func (r *repeated) String() string     { return strings.Join(*r, ",") }
func (r *repeated) Set(v string) error { *r = append(*r, v); return nil }

func runChange(root string, args []string, stdout, stderr io.Writer) error {
	if len(args) == 0 || args[0] != "begin" {
		return errors.New("usage: telos change begin --intent <id> --spec <id>")
	}
	f := flags("change begin", stderr)
	intent := f.String("intent", "", "sealed intent id")
	var specs repeated
	f.Var(&specs, "spec", "sealed spec id (repeatable)")
	if err := f.Parse(args[1:]); err != nil {
		return err
	}
	if *intent == "" {
		return errors.New("--intent is required")
	}
	id, err := beginChange(root, *intent, specs)
	if err != nil {
		return err
	}
	fmt.Fprintln(stdout, id)
	return nil
}

func runContext(root string, args []string, stdout, stderr io.Writer) error {
	f := flags("context", stderr)
	change := f.String("change", "", "change id")
	if err := f.Parse(args); err != nil {
		return err
	}
	if *change == "" {
		return errors.New("--change is required")
	}
	path, err := buildContext(root, *change)
	if err != nil {
		return err
	}
	fmt.Fprintln(stdout, path)
	return nil
}

func runVerify(root string, args []string, stdout, stderr io.Writer) error {
	f := flags("verify", stderr)
	ci := f.Bool("ci", false, "CI output mode")
	if err := f.Parse(args); err != nil {
		return err
	}
	_ = ci
	results, err := audit(root)
	if err != nil {
		return err
	}
	for _, r := range results {
		if r.Status != "ok" {
			return fmt.Errorf("%s: %s (%s)", r.Status, r.Path, r.Detail)
		}
	}
	cfg, err := readConfig(root)
	if err != nil {
		return err
	}
	if err := runVerificationCommands(root, cfg.VerificationCommands); err != nil {
		return err
	}
	lock, err := loadLock(root)
	if err != nil {
		return err
	}
	if err := appendEvent(root, "project.verified", "project", map[string]any{"commands": cfg.VerificationCommands}, lock.RootHash); err != nil {
		return err
	}
	fmt.Fprintf(stdout, "verified %d sealed artifacts; root %s\n", len(lock.Artifacts), empty(lock.RootHash, "unsealed"))
	return nil
}

func fileExists(path string) error    { _, err := os.Stat(path); return err }
func commandExists(name string) error { _, err := exec.LookPath(name); return err }
func empty(s, fallback string) string {
	if s == "" {
		return fallback
	}
	return s
}

func runGuard(stdin io.Reader, stdout io.Writer) error {
	var input map[string]any
	if err := json.NewDecoder(stdin).Decode(&input); err != nil {
		// Running guard manually should be harmless.
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
	lock, err := loadLock(root)
	if err != nil {
		return err
	}
	raw, _ := json.Marshal(input["tool_input"])
	probe := filepath.ToSlash(string(raw))
	for _, f := range lock.Artifacts {
		path := filepath.ToSlash(f.Path)
		abs := filepath.ToSlash(filepath.Join(root, filepath.FromSlash(f.Path)))
		if strings.Contains(probe, path) || strings.Contains(probe, abs) {
			response := map[string]any{"hookSpecificOutput": map[string]any{"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": "Telos sealed artifact is immutable: " + f.Path + ". Create a new revision through Telos."}}
			return json.NewEncoder(stdout).Encode(response)
		}
	}
	return nil
}

func atoi(s string) int { n, _ := strconv.Atoi(s); return n }
