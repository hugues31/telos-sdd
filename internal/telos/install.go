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
	alreadyInitialized := false
	if st, err := os.Stat(filepath.Join(root, ".telos")); err == nil && st.IsDir() {
		alreadyInitialized = true
	}
	if alreadyInitialized {
		if _, err := os.Stat(filepath.Join(root, filepath.FromSlash(repositoryLockPath))); err == nil {
			if err := requireRepositoryClean(root); err != nil {
				return err
			}
		}
	}
	if err := ensureDirs(root); err != nil {
		return err
	}
	agents := []string{agent}
	if agent == "all" {
		agents = []string{"codex", "claude"}
	}
	if !alreadyInitialized {
		cfg := Config{Agents: agents, VerificationCommands: []string{}}
		if err := atomicWrite(filepath.Join(root, ".telos", "config.toml"), []byte(configText(cfg)), 0o644); err != nil {
			return err
		}
		if err := saveLock(root, Lock{Artifacts: []LockedFile{}}); err != nil {
			return err
		}
	} else {
		cfg, err := readConfig(root)
		if err != nil {
			return err
		}
		seen := map[string]bool{}
		for _, configured := range cfg.Agents {
			seen[configured] = true
		}
		for _, requested := range agents {
			if !seen[requested] {
				cfg.Agents = append(cfg.Agents, requested)
				seen[requested] = true
			}
		}
		sort.Strings(cfg.Agents)
		if err := atomicWrite(filepath.Join(root, ".telos", "config.toml"), []byte(configText(cfg)), 0o644); err != nil {
			return err
		}
	}
	if err := rebuildState(root); err != nil {
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
		if err := atomicWrite(filepath.Join(root, ".github", "workflows", "telos-verify.yml"), data, 0o644); err != nil {
			return err
		}
	}
	eventType := "project.initialized"
	if alreadyInitialized {
		eventType = "project.adapters-refreshed"
	}
	if err := appendEvent(root, eventType, "project", map[string]any{"agents": agents}, ""); err != nil {
		return err
	}
	_, err := baselineRepository(root, "repository.baselined", "project")
	return err
}

func installAgentFiles(root, agent string) error {
	manifest := InstallManifest{Files: map[string]string{}}
	manifestPath := filepath.Join(root, ".telos", "install-manifest.json")
	if err := readJSON(manifestPath, &manifest); err != nil && !os.IsNotExist(err) {
		return err
	}
	if manifest.Files == nil {
		manifest.Files = map[string]string{}
	}
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
			if err := atomicWrite(target, data, 0o644); err != nil {
				return err
			}
			h, _ := fileHash(target)
			manifest.Files[filepath.ToSlash(strings.TrimPrefix(target, root+string(filepath.Separator)))] = h
			return nil
		})
	}
	if agent == "codex" || agent == "all" {
		if err := installSkills(".agents/skills"); err != nil {
			return err
		}
		if err := copyBundleTree(root, "adapters/codex/agents", ".codex/agents", &manifest); err != nil {
			return err
		}
		if err := mergeHookSettings(root, "hooks/codex-hooks.json", ".codex/hooks.json", &manifest); err != nil {
			return err
		}
		if err := updateInstructions(filepath.Join(root, "AGENTS.md"), codexInstructions); err != nil {
			return err
		}
		recordManagedHash(root, "AGENTS.md", &manifest)
	}
	if agent == "claude" || agent == "all" {
		if err := installSkills(".claude/skills"); err != nil {
			return err
		}
		if err := copyBundleTree(root, "adapters/claude/agents", ".claude/agents", &manifest); err != nil {
			return err
		}
		if err := mergeHookSettings(root, "hooks/claude-settings.json", ".claude/settings.json", &manifest); err != nil {
			return err
		}
		if err := updateInstructions(filepath.Join(root, "CLAUDE.md"), claudeInstructions); err != nil {
			return err
		}
		recordManagedHash(root, "CLAUDE.md", &manifest)
	}
	return writeJSON(manifestPath, manifest)
}

func recordManagedHash(root, rel string, manifest *InstallManifest) {
	h, err := fileHash(filepath.Join(root, filepath.FromSlash(rel)))
	if err == nil {
		manifest.Files[filepath.ToSlash(rel)] = h
	}
}

func copyBundleTree(root, src, dst string, manifest *InstallManifest) error {
	return fs.WalkDir(bundle.FS, src, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			return nil
		}
		rel := strings.TrimPrefix(path, src+"/")
		return copyBundleFile(root, path, filepath.ToSlash(filepath.Join(dst, rel)), manifest)
	})
}

func copyBundleFile(root, src, dst string, manifest *InstallManifest) error {
	data, err := bundle.FS.ReadFile(src)
	if err != nil {
		return err
	}
	target := filepath.Join(root, filepath.FromSlash(dst))
	if err := atomicWrite(target, data, 0o644); err != nil {
		return err
	}
	h, _ := fileHash(target)
	manifest.Files[filepath.ToSlash(dst)] = h
	return nil
}

func mergeHookSettings(root, src, dst string, manifest *InstallManifest) error {
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
	if err := atomicWrite(target, merged, 0o644); err != nil {
		return err
	}
	h, _ := fileHash(target)
	manifest.Files[filepath.ToSlash(dst)] = h
	return nil
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
- Run ` + "`telos inspect --json`" + ` first and resume its active flow. Never ask the user for Telos commands, IDs, or paths.
- Treat intents, specs, test plans, generated features, and repository writes as CLI-managed. Never edit them directly.
- Apply implementation only through ` + "`telos change apply`" + ` with RULE and SCN references.
- Stop on integrity errors. Never adopt an undeclared write or weaken a test.`

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
