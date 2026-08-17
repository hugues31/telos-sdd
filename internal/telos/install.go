package telos

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/hugues31/telos-sdd/bundle"
	"github.com/hugues31/telos-sdd/internal/kernel"
)

func initProject(root, agent string, githubCI bool) error {
	if agent != "codex" && agent != "claude" && agent != "all" {
		return fmt.Errorf("invalid agent %q", agent)
	}
	agents := []string{agent}
	if agent == "all" {
		agents = []string{"claude", "codex"}
	}
	alreadyInitialized := fileExists(filepath.Join(root, kernel.ConfigFile)) == nil
	if !alreadyInitialized {
		if err := atomicWrite(filepath.Join(root, kernel.ConfigFile), []byte(defaultConfig(agents)), 0o644); err != nil {
			return err
		}
	} else {
		if err := mergeConfigAgents(root, agents); err != nil {
			return err
		}
	}
	if fileExists(filepath.Join(root, filepath.FromSlash("spec/PRODUCT.md"))) != nil {
		if err := atomicWrite(filepath.Join(root, filepath.FromSlash("spec/PRODUCT.md")), []byte(productSkeleton), 0o644); err != nil {
			return err
		}
	}
	if err := ensureGitignore(root); err != nil {
		return err
	}
	if err := installAgentFiles(root, agent); err != nil {
		return err
	}
	if githubCI {
		data, err := bundle.FS.ReadFile("templates/telos-verify.yml")
		if err != nil {
			return err
		}
		// The initializing binary pins its consumers: their CI never tracks
		// @latest, so a new major behavior cannot silently reach them.
		pinned := strings.ReplaceAll(string(data), "TELOS_PIN", ConsumerPin)
		if err := atomicWrite(filepath.Join(root, ".github", "workflows", "telos-verify.yml"), []byte(pinned), 0o644); err != nil {
			return err
		}
	}
	return nil
}

func newProjectID() string {
	b := make([]byte, 8)
	if _, err := rand.Read(b); err != nil {
		return "telos-project"
	}
	return hex.EncodeToString(b)
}

func defaultConfig(agents []string) string {
	return `# Telos — project configuration. Edited by humans only; it is tracked and
# therefore protected content: changing it goes through a privileged Change.

project_id = "` + newProjectID() + `"

agents = ` + quoteList(agents) + `

# Commands ` + "`telos verify`" + ` runs; all must pass.
test_commands = []

# Files whose REQ-NNN references count as executable proof of a requirement.
test_files = []

# Evidence dependency-closure strategy: "go" (import graph) or "tree"
# (whole tracked tree, conservative). Omit to auto-detect from go.mod.
#closure = "tree"
`
}

const productSkeleton = `# Product

## Vision

Describe the purpose of this product and the outcome it exists to create.

## Intents

Add product intents as ` + "`### INT-001 — Title`" + ` sections. Every requirement
in the domain spec files is motivated by at least one intent.

## Constraints

## Non-goals
`

// ensureGitignore makes sure the derived-content directory is ignored:
// .telos/ holds only disposable caches in V2 (certificates live in git
// notes), so it must never enter the certified tree.
func ensureGitignore(root string) error {
	path := filepath.Join(root, ".gitignore")
	data, err := os.ReadFile(path)
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	content := string(normalize(data))
	for _, line := range strings.Split(content, "\n") {
		if strings.TrimSpace(line) == ".telos/" {
			return nil
		}
	}
	if content != "" && !strings.HasSuffix(content, "\n") {
		content += "\n"
	}
	content += ".telos/\n"
	return atomicWrite(path, []byte(content), 0o644)
}

// mergeConfigAgents rewrites only the `agents` line of an existing telos.toml,
// preserving the rest of the human-owned file byte for byte.
func mergeConfigAgents(root string, requested []string) error {
	cfg, err := kernel.ReadConfig(root)
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
	path := filepath.Join(root, kernel.ConfigFile)
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
		if err := updateInstructionsFromBundle(root, "AGENTS.md", "instructions/codex.md"); err != nil {
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
		if err := updateInstructionsFromBundle(root, "CLAUDE.md", "instructions/claude.md"); err != nil {
			return err
		}
	}
	return nil
}

// updateInstructionsFromBundle splices a bundle-authored instruction block
// into the managed section of the target file. Instruction content lives in
// bundle/instructions/ (CONTRIBUTING: provider files originate under
// bundle/), never in Go constants.
func updateInstructionsFromBundle(root, target, bundlePath string) error {
	instructions, err := bundle.FS.ReadFile(bundlePath)
	if err != nil {
		return err
	}
	return updateInstructions(filepath.Join(root, target), string(instructions))
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
				// The dedupe key is load-bearing: it is how re-install
				// replaces previously installed guard hooks.
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
