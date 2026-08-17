// gen-bundle renders the single-source role definitions (roles.go) into the
// provider adapters and regenerates the error-code table of the agent
// protocol from the CLI's registry. Run via `go generate ./bundle`; CI
// regenerates and fails on any diff, so generated files can never drift from
// their source.
package main

import (
	"flag"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"regexp"
	"strings"

	"github.com/hugues31/telos-sdd/internal/telos"
)

func main() {
	check := flag.Bool("check", false, "validate the bundle instead of writing it")
	flag.Parse()
	root, err := bundleRoot()
	if err != nil {
		fatal(err)
	}
	if *check {
		if errs := validate(root); len(errs) > 0 {
			for _, e := range errs {
				fmt.Fprintln(os.Stderr, "validate-bundle:", e)
			}
			os.Exit(1)
		}
		fmt.Println("bundle valid")
		return
	}
	for _, r := range roles {
		if err := write(filepath.Join(root, "adapters", "claude", "agents", r.Name+".md"), claudeAdapter(r)); err != nil {
			fatal(err)
		}
		if err := write(filepath.Join(root, "adapters", "codex", "agents", r.Name+".toml"), codexAdapter(r)); err != nil {
			fatal(err)
		}
	}
	if err := regenerateCodes(filepath.Join(root, "skills", "telos", "references", "protocol.md")); err != nil {
		fatal(err)
	}
	fmt.Println("bundle generated")
}

// readNormalized reads a file with line endings normalized, so CRLF
// checkouts (Windows runners) never masquerade as content differences.
func readNormalized(path string) (string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	return strings.ReplaceAll(string(data), "\r\n", "\n"), nil
}

// validate checks generated-file parity plus the cross-checks that keep the
// agent-facing prose honest against the CLI: every referenced verb exists,
// every referenced error code exists, and every code is documented.
func validate(root string) []string {
	var errs []string

	// Parity: adapters must equal a fresh render of roles.go. Line endings
	// are normalized first so a CRLF checkout (Windows runners) does not
	// masquerade as drift.
	for _, r := range roles {
		for _, pair := range [][2]string{
			{filepath.Join(root, "adapters", "claude", "agents", r.Name+".md"), claudeAdapter(r)},
			{filepath.Join(root, "adapters", "codex", "agents", r.Name+".toml"), codexAdapter(r)},
		} {
			data, err := readNormalized(pair[0])
			if err != nil || data != pair[1] {
				errs = append(errs, pair[0]+" drifted from roles.go; run `go generate ./bundle`")
			}
		}
	}

	// Gather bundle prose.
	var prose strings.Builder
	_ = filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}
		if strings.HasSuffix(path, ".md") || strings.HasSuffix(path, ".toml") || strings.HasSuffix(path, ".yaml") {
			data, _ := readNormalized(path)
			prose.WriteString(data)
			prose.WriteString("\n")
		}
		return nil
	})
	text := prose.String()

	// Every `telos <verb>` reference names a real command.
	known := map[string]bool{}
	for _, c := range commands {
		known[c] = true
	}
	verbRef := regexp.MustCompile("`telos ([a-z]+)")
	for _, m := range verbRef.FindAllStringSubmatch(text, -1) {
		if !known[m[1]] {
			errs = append(errs, "bundle references unknown command `telos "+m[1]+"`")
		}
	}

	// Every referenced code exists; every code is documented in protocol.md.
	registry := map[string]bool{}
	for _, c := range telos.Codes {
		registry[c.Name] = true
	}
	codeRef := regexp.MustCompile(`TELOS_[A-Z_]+`)
	for _, m := range codeRef.FindAllString(text, -1) {
		if !registry[m] {
			errs = append(errs, "bundle references unknown code "+m)
		}
	}
	protocol, err := readNormalized(filepath.Join(root, "skills", "telos", "references", "protocol.md"))
	if err != nil {
		errs = append(errs, "protocol.md unreadable")
	} else {
		for _, c := range telos.Codes {
			if !strings.Contains(protocol, "`"+c.Name+"`") {
				errs = append(errs, "protocol.md misses code "+c.Name)
			}
		}
	}

	// Skill frontmatter and the Codex skill metadata keep their V1 contracts.
	skill, err := readNormalized(filepath.Join(root, "skills", "telos", "SKILL.md"))
	if err != nil || !strings.HasPrefix(skill, "---\nname: telos\n") {
		errs = append(errs, "SKILL.md frontmatter must start with `name: telos`")
	}
	openai, err := readNormalized(filepath.Join(root, "skills", "telos", "agents", "openai.yaml"))
	if err != nil {
		errs = append(errs, "skills/telos/agents/openai.yaml missing")
	} else {
		for _, want := range []string{"display_name:", "short_description:", "default_prompt:", "$telos"} {
			if !strings.Contains(openai, want) {
				errs = append(errs, "openai.yaml misses "+want)
			}
		}
	}
	return errs
}

func bundleRoot() (string, error) {
	dir, err := os.Getwd()
	if err != nil {
		return "", err
	}
	for {
		candidate := filepath.Join(dir, "bundle")
		if st, err := os.Stat(candidate); err == nil && st.IsDir() {
			return candidate, nil
		}
		next := filepath.Dir(dir)
		if next == dir {
			return "", fmt.Errorf("bundle/ directory not found upward of the working directory")
		}
		dir = next
	}
}

func claudeAdapter(r role) string {
	return "---\nname: " + r.Name + "\ndescription: " + r.Description + "\ntools: " + r.Tools + "\nmodel: inherit\n---\n\n" + r.Body + "\n"
}

func codexAdapter(r role) string {
	// The one transform V1 did by hand: soften markdown fences for TOML prose.
	body := strings.ReplaceAll(r.Body, "```", "'''")
	return "name = " + quote(r.Name) + "\ndescription = " + quote(r.Description) + "\nsandbox_mode = " + quote(r.Sandbox) + "\ndeveloper_instructions = \"\"\"\n" + body + "\n\"\"\"\n"
}

func quote(s string) string {
	return `"` + strings.ReplaceAll(s, `"`, `\"`) + `"`
}

var codesRegion = regexp.MustCompile(`(?s)<!-- codes:begin -->.*<!-- codes:end -->`)

func regenerateCodes(protocolPath string) error {
	data, err := os.ReadFile(protocolPath)
	if err != nil {
		return err
	}
	var table strings.Builder
	table.WriteString("<!-- codes:begin -->\n")
	table.WriteString("| Code | Agent action |\n| --- | --- |\n")
	for _, c := range telos.Codes {
		table.WriteString("| `" + c.Name + "` | " + c.AgentAction + " |\n")
	}
	table.WriteString("<!-- codes:end -->")
	if !codesRegion.Match(data) {
		return fmt.Errorf("%s misses the <!-- codes:begin/end --> markers", protocolPath)
	}
	return os.WriteFile(protocolPath, codesRegion.ReplaceAll(data, []byte(table.String())), 0o644)
}

func write(path, content string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	return os.WriteFile(path, []byte(content), 0o644)
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, "gen-bundle:", err)
	os.Exit(1)
}
