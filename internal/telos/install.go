package telos

import (
	"encoding/json"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/hugues31/telos-sdd/bundle"
)

func initProject(root, agent string, githubCI bool) error {
	if agent != "codex" && agent != "claude" && agent != "all" {
		return fmt.Errorf("invalid agent %q", agent)
	}
	agents := []string{agent}
	if agent == "all" {
		agents = []string{"claude", "codex"}
	}
	alreadyInitialized := fileExists(filepath.Join(root, configFile)) == nil
	if !alreadyInitialized {
		if err := atomicWrite(filepath.Join(root, configFile), []byte(defaultConfig(agents)), 0o644); err != nil {
			return err
		}
	} else {
		if err := mergeConfigAgents(root, agents); err != nil {
			return err
		}
	}
	if fileExists(filepath.Join(root, filepath.FromSlash(productFile))) != nil {
		if err := atomicWrite(filepath.Join(root, filepath.FromSlash(productFile)), []byte(productSkeleton), 0o644); err != nil {
			return err
		}
	}
	if err := installAgentFiles(root, agent); err != nil {
		return err
	}
	if githubCI {
		data, err := bundle.FS.ReadFile("templates/telos-verify.yml")
		if err != nil {
			return err
		}
		if err := atomicWrite(filepath.Join(root, ".github", "workflows", "telos-verify.yml"), data, 0o644); err != nil {
			return err
		}
	}
	code, spec, err := inventories(root)
	if err != nil {
		return err
	}
	return saveState(root, State{Version: 1, Spec: snapshotOf(spec), Code: snapshotOf(code)})
}

func defaultConfig(agents []string) string {
	return `# Telos SDD — project configuration. Edited by humans only; the agent broker
# may never write this file.

agents = ` + quoteList(agents) + `

# Commands ` + "`telos verify`" + ` runs; all must pass.
test_commands = []

# Files whose RULE-NNN references count as executable proof of a rule.
test_files = []

# Files allowed to exist without tracing to a rule (still integrity-checked).
untraced = ["README.md", "LICENSE", ".gitignore", ".github/**", ".claude/**", ".codex/**", ".agents/**", "CLAUDE.md", "AGENTS.md", "go.mod", "go.sum", "package.json", "package-lock.json", "pnpm-lock.yaml"]
`
}

const productSkeleton = `# Product

## Vision

Describe the purpose of this product and the outcome it exists to create.

## Objectives

Add measurable objectives as ` + "`### OBJ-001 — Title`" + ` sections. Every rule in
the domain spec files traces to at least one objective.

## Constraints

## Non-goals
`

// mergeConfigAgents rewrites only the `agents` line of an existing telos.toml,
// preserving the rest of the human-owned file byte for byte.
func mergeConfigAgents(root string, requested []string) error {
	cfg, err := readConfig(root)
	if err != nil {
		return err
	}
	seen := map[string]bool{}
	merged := append([]string{}, cfg.Agents...)
	for _, agent := range cfg.Agents {
		seen[agent] = true
	}
	added := false
	for _, agent := range requested {
		if !seen[agent] {
			merged = append(merged, agent)
			seen[agent] = true
			added = true
		}
	}
	if !added {
		return nil
	}
	sort.Strings(merged)
	path := filepath.Join(root, configFile)
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	lines := strings.Split(string(normalize(data)), "\n")
	replaced := false
	for i, line := range lines {
		key := strings.TrimSpace(strings.SplitN(stripComment(line), "=", 2)[0])
		if key == "agents" {
			lines[i] = "agents = " + quoteList(merged)
			replaced = true
			break
		}
	}
	if !replaced {
		lines = append([]string{"agents = " + quoteList(merged)}, lines...)
	}
	return atomicWrite(path, []byte(strings.Join(lines, "\n")), 0o644)
}

func installAgentFiles(root, agent string) error {
	installSkills := func(dest string) error {
		return fs.WalkDir(bundle.FS, "skills", func(path string, d fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if d.IsDir() {
				return nil
			}
			rel := strings.TrimPrefix(path, "skills/")
			data, err := bundle.FS.ReadFile(path)
			if err != nil {
				return err
			}
			target := filepath.Join(root, filepath.FromSlash(dest), filepath.FromSlash(rel))
			return atomicWrite(target, data, 0o644)
		})
	}
	if agent == "codex" || agent == "all" {
		if err := installSkills(".agents/skills"); err != nil {
			return err
		}
		if err := copyBundleTree(root, "adapters/codex/agents", ".codex/agents"); err != nil {
			return err
		}
		if err := mergeHookSettings(root, "hooks/codex-hooks.json", ".codex/hooks.json"); err != nil {
			return err
		}
		if err := updateInstructions(filepath.Join(root, "AGENTS.md"), codexInstructions); err != nil {
			return err
		}
	}
	if agent == "claude" || agent == "all" {
		if err := installSkills(".claude/skills"); err != nil {
			return err
		}
		if err := copyBundleTree(root, "adapters/claude/agents", ".claude/agents"); err != nil {
			return err
		}
		if err := mergeHookSettings(root, "hooks/claude-settings.json", ".claude/settings.json"); err != nil {
			return err
		}
		if err := updateInstructions(filepath.Join(root, "CLAUDE.md"), claudeInstructions); err != nil {
			return err
		}
	}
	return nil
}

func copyBundleTree(root, src, dst string) error {
	return fs.WalkDir(bundle.FS, src, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			return nil
		}
		rel := strings.TrimPrefix(path, src+"/")
		data, err := bundle.FS.ReadFile(path)
		if err != nil {
			return err
		}
		target := filepath.Join(root, filepath.FromSlash(dst), filepath.FromSlash(rel))
		return atomicWrite(target, data, 0o644)
	})
}

func mergeHookSettings(root, src, dst string) error {
	generated, err := bundle.FS.ReadFile(src)
	if err != nil {
		return err
	}
	target := filepath.Join(root, filepath.FromSlash(dst))
	existing, err := os.ReadFile(target)
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	merged, err := mergeSettings(existing, generated)
	if err != nil {
		return fmt.Errorf("merge %s: %w", dst, err)
	}
	return atomicWrite(target, merged, 0o644)
}

func updateInstructions(path, instructions string) error {
	data, err := os.ReadFile(path)
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	return atomicWrite(path, []byte(managed(string(data), instructions)), 0o644)
}

const codexInstructions = `# Telos SDD

- Use the ` + "`$telos`" + ` Skill for every feature, bug fix, refactor, or repository modification.
- Run ` + "`telos status --json`" + ` first and act on its phase and next actions.
- The spec under spec/ is the source of intent. Never edit it directly: use ` + "`telos spec put`" + `, present ` + "`telos spec review`" + `, and let the human approve.
- Apply code only through ` + "`telos apply --rule RULE-NNN`" + `; every touched file carries a ` + "`telos:`" + ` annotation and every rule gets a tagged test.
- Stop on TELOS_CODE_CORRUPTED. Never adopt an out-of-band code edit or weaken a test.`

const claudeInstructions = `# Telos SDD

@AGENTS.md

Use the project ` + "`/telos`" + ` Skill as the sole user-facing workflow and delegate its specialized agents from .claude/agents. Respect the Telos strict guard hook.`

func mergeSettings(existing []byte, generated []byte) ([]byte, error) {
	var a, b map[string]any
	if len(existing) > 0 && json.Unmarshal(existing, &a) != nil {
		return nil, fmt.Errorf("existing hook settings are not valid JSON")
	}
	if err := json.Unmarshal(generated, &b); err != nil {
		return nil, err
	}
	if a == nil {
		a = map[string]any{}
	}
	for k, v := range b {
		if k != "hooks" {
			if _, exists := a[k]; !exists {
				a[k] = v
			}
			continue
		}
		generatedHooks, _ := v.(map[string]any)
		existingHooks, _ := a[k].(map[string]any)
		if existingHooks == nil {
			existingHooks = map[string]any{}
		}
		for event, groups := range generatedHooks {
			current, _ := existingHooks[event].([]any)
			filtered := current[:0]
			for _, item := range current {
				encoded, _ := json.Marshal(item)
				if !strings.Contains(string(encoded), "telos guard") {
					filtered = append(filtered, item)
				}
			}
			current = filtered
			for _, group := range groups.([]any) {
				current = append(current, group)
			}
			existingHooks[event] = current
		}
		a[k] = existingHooks
	}
	out, err := json.MarshalIndent(a, "", "  ")
	return append(out, '\n'), err
}
