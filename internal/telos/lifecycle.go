package telos

import (
	"errors"
	"fmt"
	"io"
	"math/rand"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"runtime"
	"sort"
	"strings"
	"time"
)

var brainstormEngines = []string{
	"SCAMPER", "Reverse brainstorming", "Six thinking hats", "Assumption reversal",
	"Morphological matrix", "Jobs to be Done", "Pre-mortem", "First principles",
	"Constraint removal", "Analogical transfer", "Worst possible idea", "Impact/effort convergence",
}

func startBrainstorm(root, mode string, seed int64) (string, string, error) {
	if mode != "choose" && mode != "recommend" && mode != "random" && mode != "progressive" {
		return "", "", fmt.Errorf("invalid mode %q", mode)
	}
	if seed == 0 {
		seed = time.Now().UTC().UnixNano()
	}
	engine := selectBrainstormEngine(mode, seed)
	id, err := newID("brn", time.Now())
	if err != nil {
		return "", "", err
	}
	meta := ArtifactMeta{ID: id, Kind: "brainstorm", Status: "exploring", Revision: 1}
	body := fmt.Sprintf("# Brainstorm\n\n## Prompt\n\nDescribe the opportunity or problem.\n\n## Engine\n\n%s\n\n## Seed\n\n%d\n\n## Divergence\n\nCapture ideas without judging them.\n\n## Convergence\n\nRank ideas by evidence, impact, effort, and reversibility.\n\n## Promotion candidate\n\nState the idea to promote into an intent, or `None.`\n", engine, seed)
	rel := filepath.ToSlash(filepath.Join(".telos", "brainstorms", strings.ToLower(id)+".md"))
	if err := atomicWrite(filepath.Join(root, filepath.FromSlash(rel)), renderArtifact(meta, body), 0o644); err != nil {
		return "", "", err
	}
	if err := appendEvent(root, "brainstorm.started", id, map[string]any{"mode": mode, "engine": engine, "seed": seed, "path": rel}, ""); err != nil {
		return "", "", err
	}
	return id, rel, nil
}

func selectBrainstormEngine(mode string, seed int64) string {
	r := rand.New(rand.NewSource(seed))
	engine := brainstormEngines[0]
	switch mode {
	case "random":
		engine = brainstormEngines[r.Intn(len(brainstormEngines))]
	case "recommend":
		engine = "First principles"
	case "progressive":
		engine = "Jobs to be Done → Assumption reversal → Impact/effort convergence"
	case "choose":
		engine = "Choose one: " + strings.Join(brainstormEngines, ", ")
	}
	return engine
}

func newIntent(root, title, from string) (string, string, error) {
	if from != "" {
		_, _, brainstorm, err := findArtifact(root, "brainstorm", from)
		if err != nil {
			return "", "", err
		}
		candidate := sectionText(brainstorm, "## Promotion candidate")
		if candidate == "" || candidate == "None." {
			return "", "", errors.New("brainstorm has no explicit promotion candidate")
		}
	}
	id, err := newID("int", time.Now())
	if err != nil {
		return "", "", err
	}
	if strings.TrimSpace(title) == "" {
		title = "Untitled intent"
	}
	parents := []string{}
	if from != "" {
		parents = append(parents, from)
	}
	meta := ArtifactMeta{ID: id, Kind: "intent", Status: "draft", Revision: 1, Parents: parents}
	body := fmt.Sprintf("# %s\n\n## Outcome\n\nTODO: Describe the observable outcome, not the implementation.\n\n## Actors\n\nTODO: Name actors and permissions.\n\n## Scope\n\nTODO: State included behavior.\n\n## Non-goals\n\nTODO: State exclusions explicitly.\n\n## Success criteria\n\n### CRIT-001 — Observable criterion\n\nTODO: Provide one measurable criterion.\n\n## Constraints\n\nTODO: State technical, legal, performance, and compatibility constraints.\n\n## Open questions\n\nTODO: Resolve every material ambiguity; write `None.` only when resolved.\n", title)
	rel := filepath.ToSlash(filepath.Join(".telos", "intents", strings.ToLower(id)+".md"))
	if err := atomicWrite(filepath.Join(root, filepath.FromSlash(rel)), renderArtifact(meta, body), 0o644); err != nil {
		return "", "", err
	}
	if err := appendEvent(root, "intent.created", id, map[string]any{"path": rel, "from": from}, ""); err != nil {
		return "", "", err
	}
	return id, rel, nil
}

func newSpec(root, intent, title string) (string, string, error) {
	_, im, _, err := findArtifact(root, "intent", intent)
	if err != nil {
		return "", "", err
	}
	if im.Status != "sealed" {
		return "", "", errors.New("intent must be sealed before deriving a spec")
	}
	if strings.TrimSpace(title) == "" {
		title = "Behavioral specification"
	}
	id, err := newID("spc", time.Now())
	if err != nil {
		return "", "", err
	}
	meta := ArtifactMeta{ID: id, Kind: "spec", Status: "draft", Revision: 1, Intent: intent, Parents: []string{intent}}
	body := fmt.Sprintf("# %s\n\n## Context\n\nTODO: Define the state and actors this behavior applies to.\n\n## Rules\n\n### RULE-001 — Name\n\nTraces: CRIT-001\n\nTODO: Write one normative, observable rule.\n\n## Examples\n\nTODO: Include positive, negative, boundary, permission, and failure examples.\n\n## Boundaries\n\nTODO: Define limits, empty values, concurrency, retries, and idempotency where relevant.\n\n## Non-effects\n\nTODO: Define what must not change or happen.\n\n## Failure modes\n\nTODO: Define errors, recovery, and prohibited partial effects.\n\n## Observability\n\nTODO: Define externally observable signals and audit evidence.\n", title)
	rel := filepath.ToSlash(filepath.Join(".telos", "specs", strings.ToLower(id)+".md"))
	if err := atomicWrite(filepath.Join(root, filepath.FromSlash(rel)), renderArtifact(meta, body), 0o644); err != nil {
		return "", "", err
	}
	if err := appendEvent(root, "spec.created", id, map[string]any{"path": rel, "intent": intent}, ""); err != nil {
		return "", "", err
	}
	return id, rel, nil
}

func sealArtifact(root, kind, id string) error {
	path, meta, body, err := findArtifact(root, kind, id)
	if err != nil {
		return err
	}
	if meta.Status == "sealed" {
		return errors.New("artifact is already sealed")
	}
	if err := validateBody(kind, body); err != nil {
		return err
	}
	if kind == "spec" {
		_, parent, _, err := findArtifact(root, "intent", meta.Intent)
		if err != nil {
			return err
		}
		if parent.Status != "sealed" {
			return errors.New("parent intent is not sealed")
		}
	}
	meta.Status = "sealed"
	if err := atomicWrite(path, renderArtifact(meta, body), 0o444); err != nil {
		return err
	}
	h, err := fileHash(path)
	if err != nil {
		return err
	}
	if err := storeBlob(root, path, h); err != nil {
		return err
	}
	rel := relative(root, path)
	lock, err := lockFile(root, LockedFile{ID: meta.ID, Kind: meta.Kind, Path: rel, Hash: h, Parents: meta.Parents})
	if err != nil {
		return err
	}
	return appendEvent(root, kind+".sealed", id, map[string]any{"path": rel, "hash": h}, lock.RootHash)
}

func renderFeature(plan TestPlan) string {
	var b strings.Builder
	b.WriteString("# Generated by Telos. Edit the test plan, never this file.\n")
	fmt.Fprintf(&b, "@spec_%s\nFeature: %s\n", tag(plan.Spec), human(plan.Feature))
	for _, s := range plan.Scenarios {
		b.WriteByte('\n')
		tags := append([]string(nil), s.Tags...)
		tags = append(tags, strings.ToLower(s.Rule), strings.ToLower(s.ID))
		sort.Strings(tags)
		for _, t := range tags {
			fmt.Fprintf(&b, "@%s ", tag(t))
		}
		b.WriteByte('\n')
		fmt.Fprintf(&b, "Scenario: %s\n", s.Name)
		writeSteps := func(keyword string, steps []string) {
			for i, step := range steps {
				k := keyword
				if i > 0 {
					k = "And"
				}
				fmt.Fprintf(&b, "  %s %s\n", k, step)
			}
		}
		writeSteps("Given", s.Given)
		writeSteps("When", s.When)
		writeSteps("Then", s.Then)
	}
	return b.String()
}

func beginChange(root, intent string, specs []string) (string, error) {
	if err := requireCleanAudit(root); err != nil {
		return "", err
	}
	lock, err := loadLock(root)
	if err != nil {
		return "", err
	}
	_, im, _, err := findArtifact(root, "intent", intent)
	if err != nil {
		return "", err
	}
	if im.Status != "sealed" {
		return "", errors.New("intent must be sealed")
	}
	if len(specs) == 0 {
		return "", errors.New("at least one --spec is required")
	}
	for _, id := range specs {
		_, sm, _, err := findArtifact(root, "spec", id)
		if err != nil {
			return "", err
		}
		if sm.Status != "sealed" || sm.Intent != intent {
			return "", fmt.Errorf("spec %s is not sealed under intent %s", id, intent)
		}
		if !artifactIDInLock(lock, id+":plan") || !artifactIDInLock(lock, id+":feature") {
			return "", fmt.Errorf("spec %s is not part of an atomically sealed executable contract", id)
		}
	}
	id, err := newID("chg", time.Now())
	if err != nil {
		return "", err
	}
	base := "unborn"
	if out, err := exec.Command("git", "-C", root, "rev-parse", "HEAD").Output(); err == nil {
		base = strings.TrimSpace(string(out))
	}
	repository, err := loadRepositoryLock(root)
	if os.IsNotExist(err) {
		repository, err = baselineRepository(root, "repository.baselined", "project")
	}
	if err != nil {
		return "", err
	}
	change := Change{ID: id, Intent: intent, Specs: specs, Base: base, Status: "active", Started: time.Now().UTC().Format(time.RFC3339), SourceBaseRoot: repository.RootHash, SourceCurrentRoot: repository.RootHash}
	path := filepath.Join(root, ".telos", "changes", strings.ToLower(id)+".json")
	if err := writeJSON(path, change); err != nil {
		return "", err
	}
	if err := appendEvent(root, "change.started", id, map[string]any{"intent": intent, "specs": specs, "base": base}, ""); err != nil {
		return "", err
	}
	return id, nil
}

func beginFlowChange(root, flowID string) (Flow, Change, error) {
	flow, err := loadFlow(root, flowID)
	if err != nil {
		return flow, Change{}, err
	}
	if flow.Phase != "ready_to_implement" || flow.Intent == "" || len(flow.Specs) == 0 {
		return flow, Change{}, coded("TELOS_PHASE_INVALID", "flow contract is not sealed and ready to implement")
	}
	id, err := beginChange(root, flow.Intent, flow.Specs)
	if err != nil {
		return flow, Change{}, err
	}
	path := filepath.Join(root, ".telos", "changes", strings.ToLower(id)+".json")
	var change Change
	if err := readJSON(path, &change); err != nil {
		return flow, change, err
	}
	change.Flow = flow.ID
	if err := writeJSON(path, change); err != nil {
		return flow, change, err
	}
	flow.Change = id
	flow.Phase = "implementing"
	if err := saveFlow(root, flow); err != nil {
		return flow, change, err
	}
	return flow, change, nil
}

func buildContext(root, changeID string) (string, error) {
	path := filepath.Join(root, ".telos", "changes", strings.ToLower(changeID)+".json")
	var change Change
	if err := readJSON(path, &change); err != nil {
		return "", err
	}
	lock, err := loadLock(root)
	if err != nil {
		return "", err
	}
	var b strings.Builder
	fmt.Fprintf(&b, "# Telos implementation context\n\nChange: `%s`\nBase: `%s`\n\n", change.ID, change.Base)
	ip, _, _, err := findArtifact(root, "intent", change.Intent)
	if err != nil {
		return "", err
	}
	idata, _ := os.ReadFile(ip)
	b.WriteString("## Sealed intent\n\n")
	b.Write(normalize(idata))
	b.WriteString("\n")
	for _, spec := range change.Specs {
		sp, _, _, err := findArtifact(root, "spec", spec)
		if err != nil {
			return "", err
		}
		data, _ := os.ReadFile(sp)
		b.WriteString("## Sealed specification\n\n")
		b.Write(normalize(data))
		b.WriteString("\n")
		feature := ""
		for _, artifact := range lock.Artifacts {
			if artifact.ID == spec+":feature" {
				feature = filepath.Join(root, filepath.FromSlash(artifact.Path))
				break
			}
		}
		if data, err := os.ReadFile(feature); feature != "" && err == nil {
			b.WriteString("## Executable scenarios\n\n```gherkin\n")
			b.Write(normalize(data))
			b.WriteString("```\n\n")
		}
	}
	b.WriteString("## Implementation contract\n\n- Implement only the rules and scenarios above.\n- Do not edit sealed artifacts or generated features.\n- Do not weaken tests.\n- Run `telos verify` before completion.\n")
	out := filepath.Join(root, ".telos", "context.md")
	if err := atomicWrite(out, []byte(b.String()), 0o644); err != nil {
		return "", err
	}
	h, err := fileHash(out)
	if err != nil {
		return "", err
	}
	if err := storeBlob(root, out, h); err != nil {
		return "", err
	}
	change.ContextHash = h
	if err := writeJSON(path, change); err != nil {
		return "", err
	}
	return relative(root, out), nil
}

func runVerificationCommands(root string, commands []string, stdout, stderr io.Writer) error {
	for _, command := range commands {
		var cmd *exec.Cmd
		if runtime.GOOS == "windows" {
			cmd = exec.Command("cmd", "/C", command)
		} else {
			cmd = exec.Command("sh", "-c", command)
		}
		cmd.Dir = root
		cmd.Stdout, cmd.Stderr = stdout, stderr
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("verification command failed (%s): %w", command, err)
		}
	}
	return nil
}

func slug(s string) string {
	s = strings.ToLower(strings.TrimSpace(s))
	re := regexp.MustCompile(`[^a-z0-9]+`)
	s = strings.Trim(re.ReplaceAllString(s, "-"), "-")
	if s == "" {
		return "feature"
	}
	return s
}

func tag(s string) string {
	return strings.ReplaceAll(slug(s), "-", "_")
}

func human(s string) string {
	return strings.TrimSpace(strings.ReplaceAll(s, "-", " "))
}

func relative(root, path string) string {
	rel, _ := filepath.Rel(root, path)
	return filepath.ToSlash(rel)
}
