package kernel

import (
	"bufio"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/hugues31/telos-sdd/internal/coded"
)

// ConfigFile is the project configuration at the repository root. It is
// git-tracked and therefore protected content: changing it certifiably is a
// privileged transition (KERNEL-009).
const ConfigFile = "telos.toml"

// Config is the human-owned project configuration.
type Config struct {
	ProjectID    string
	Agents       []string
	TestCommands []string
	TestFiles    []string
	// Closure selects the evidence dependency-closure strategy: "go"
	// (import-graph closure), "tree" (whole tracked tree, conservative), or
	// "" (auto: "go" when go.mod exists).
	Closure string
}

// EffectiveClosure resolves the closure strategy for a repository root.
func (c Config) EffectiveClosure(root string) string {
	if c.Closure != "" {
		return c.Closure
	}
	if _, err := os.Stat(filepath.Join(root, "go.mod")); err == nil {
		return "go"
	}
	return "tree"
}

// ReadConfig parses telos.toml at root. A missing file means the project is
// not initialized.
func ReadConfig(root string) (Config, error) {
	cfg := Config{}
	f, err := os.Open(filepath.Join(root, ConfigFile))
	if err != nil {
		return cfg, coded.New("TELOS_NOT_INITIALIZED", "telos.toml is missing; run `telos init`")
	}
	defer f.Close()
	s := bufio.NewScanner(f)
	for s.Scan() {
		line := strings.TrimSpace(stripComment(s.Text()))
		kv := strings.SplitN(line, "=", 2)
		if len(kv) != 2 {
			continue
		}
		key, val := strings.TrimSpace(kv[0]), strings.TrimSpace(kv[1])
		for strings.HasPrefix(val, "[") && !strings.HasSuffix(val, "]") && s.Scan() {
			val += " " + strings.TrimSpace(stripComment(s.Text()))
		}
		switch key {
		case "project_id":
			cfg.ProjectID = unquoteScalar(val)
		case "agents":
			cfg.Agents = parseList(val)
		case "test_commands":
			cfg.TestCommands = parseList(val)
		case "test_files":
			cfg.TestFiles = parseList(val)
		case "closure":
			cfg.Closure = unquoteScalar(val)
		default:
			return cfg, coded.New("TELOS_CONFIG_INVALID", "unknown key "+strconv.Quote(key)+" in telos.toml; valid keys: project_id, agents, test_commands, test_files, closure")
		}
	}
	if err := s.Err(); err != nil {
		return cfg, coded.New("TELOS_CONFIG_INVALID", "telos.toml is unreadable: "+err.Error())
	}
	if cfg.Closure != "" && cfg.Closure != "go" && cfg.Closure != "tree" {
		return cfg, coded.New("TELOS_CONFIG_INVALID", "closure must be \"go\" or \"tree\", got "+strconv.Quote(cfg.Closure))
	}
	return cfg, nil
}

func stripComment(line string) string {
	return strings.SplitN(line, "#", 2)[0]
}

func unquoteScalar(s string) string {
	s = strings.TrimSpace(s)
	if v, err := strconv.Unquote(s); err == nil {
		return v
	}
	return s
}

func parseList(s string) []string {
	s = strings.TrimSpace(s)
	if !strings.HasPrefix(s, "[") || !strings.HasSuffix(s, "]") {
		return nil
	}
	s = strings.TrimSpace(strings.TrimSuffix(strings.TrimPrefix(s, "["), "]"))
	if s == "" {
		return []string{}
	}
	var out []string
	for _, part := range strings.Split(s, ",") {
		part = strings.TrimSpace(part)
		if part == "" {
			continue
		}
		if v, err := strconv.Unquote(part); err == nil {
			out = append(out, v)
		}
	}
	return out
}
