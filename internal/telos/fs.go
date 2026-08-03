package telos

import (
	"bufio"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"
)

func findRoot(start string) (string, error) {
	p, err := filepath.Abs(start)
	if err != nil {
		return "", err
	}
	for {
		if st, err := os.Stat(filepath.Join(p, ".telos")); err == nil && st.IsDir() {
			return p, nil
		}
		next := filepath.Dir(p)
		if next == p {
			return "", errors.New("not inside a Telos project; run `telos init`")
		}
		p = next
	}
}

func ensureDirs(root string) error {
	dirs := []string{
		".telos/brainstorms", ".telos/intents", ".telos/specs", ".telos/test-plans",
		".telos/changes", ".telos/ledger/events", "features",
	}
	for _, dir := range dirs {
		if err := os.MkdirAll(filepath.Join(root, filepath.FromSlash(dir)), 0o755); err != nil {
			return err
		}
	}
	return nil
}

func atomicWrite(path string, data []byte, mode fs.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	f, err := os.CreateTemp(filepath.Dir(path), ".telos-tmp-*")
	if err != nil {
		return err
	}
	tmp := f.Name()
	defer os.Remove(tmp)
	if _, err = f.Write(data); err != nil {
		f.Close()
		return err
	}
	if err = f.Chmod(mode); err != nil {
		f.Close()
		return err
	}
	if err = f.Close(); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

func writeJSON(path string, v any) error {
	b, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return err
	}
	b = append(b, '\n')
	return atomicWrite(path, b, 0o644)
}

func readJSON(path string, v any) error {
	b, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	return json.Unmarshal(b, v)
}

func normalize(data []byte) []byte {
	s := strings.ReplaceAll(string(data), "\r\n", "\n")
	s = strings.ReplaceAll(s, "\r", "\n")
	return []byte(s)
}

func fileHash(path string) (string, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	sum := sha256.Sum256(normalize(b))
	return hex.EncodeToString(sum[:]), nil
}

func rootHash(files []LockedFile) string {
	ordered := append([]LockedFile(nil), files...)
	sort.Slice(ordered, func(i, j int) bool { return ordered[i].Path < ordered[j].Path })
	h := sha256.New()
	for _, f := range ordered {
		io.WriteString(h, filepath.ToSlash(f.Path))
		h.Write([]byte{0})
		io.WriteString(h, f.Hash)
		h.Write([]byte{'\n'})
	}
	return hex.EncodeToString(h.Sum(nil))
}

func newID(prefix string, now time.Time) (string, error) {
	b := make([]byte, 3)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return strings.ToUpper(prefix) + "-" + now.UTC().Format("20060102") + "-" + strings.ToUpper(hex.EncodeToString(b)), nil
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
		if v, err := strconv.Unquote(part); err == nil {
			out = append(out, v)
		}
	}
	return out
}

func quoteList(v []string) string {
	parts := make([]string, len(v))
	for i, s := range v {
		parts[i] = strconv.Quote(s)
	}
	return "[" + strings.Join(parts, ", ") + "]"
}

func readConfig(root string) (Config, error) {
	cfg := Config{Version: configVersion, Profile: "standard"}
	f, err := os.Open(filepath.Join(root, ".telos", "config.toml"))
	if err != nil {
		return cfg, err
	}
	defer f.Close()
	s := bufio.NewScanner(f)
	for s.Scan() {
		line := strings.TrimSpace(strings.SplitN(s.Text(), "#", 2)[0])
		kv := strings.SplitN(line, "=", 2)
		if len(kv) != 2 {
			continue
		}
		key, val := strings.TrimSpace(kv[0]), strings.TrimSpace(kv[1])
		switch key {
		case "version":
			cfg.Version, _ = strconv.Atoi(val)
		case "profile":
			cfg.Profile, _ = strconv.Unquote(val)
		case "agents":
			cfg.Agents = parseList(val)
		case "verification_commands":
			cfg.VerificationCommands = parseList(val)
		}
	}
	return cfg, s.Err()
}

func configText(cfg Config) string {
	return fmt.Sprintf("version = %d\nprofile = %q\nagents = %s\nverification_commands = %s\n", cfg.Version, cfg.Profile, quoteList(cfg.Agents), quoteList(cfg.VerificationCommands))
}

func parseArtifact(data []byte) (ArtifactMeta, string, error) {
	text := string(normalize(data))
	if !strings.HasPrefix(text, "+++\n") {
		return ArtifactMeta{}, "", errors.New("missing TOML frontmatter")
	}
	end := strings.Index(text[4:], "\n+++\n")
	if end < 0 {
		return ArtifactMeta{}, "", errors.New("unterminated TOML frontmatter")
	}
	head := text[4 : 4+end]
	body := text[4+end+5:]
	meta := ArtifactMeta{}
	for _, line := range strings.Split(head, "\n") {
		kv := strings.SplitN(line, "=", 2)
		if len(kv) != 2 {
			continue
		}
		key, val := strings.TrimSpace(kv[0]), strings.TrimSpace(kv[1])
		switch key {
		case "id":
			meta.ID, _ = strconv.Unquote(val)
		case "type":
			meta.Kind, _ = strconv.Unquote(val)
		case "status":
			meta.Status, _ = strconv.Unquote(val)
		case "revision":
			meta.Revision, _ = strconv.Atoi(val)
		case "intent":
			meta.Intent, _ = strconv.Unquote(val)
		case "parents":
			meta.Parents = parseList(val)
		}
	}
	if meta.ID == "" || meta.Kind == "" {
		return meta, body, errors.New("frontmatter requires id and type")
	}
	return meta, body, nil
}

func renderArtifact(meta ArtifactMeta, body string) []byte {
	var b strings.Builder
	b.WriteString("+++\n")
	fmt.Fprintf(&b, "id = %q\ntype = %q\nstatus = %q\nrevision = %d\n", meta.ID, meta.Kind, meta.Status, meta.Revision)
	if meta.Intent != "" {
		fmt.Fprintf(&b, "intent = %q\n", meta.Intent)
	}
	if len(meta.Parents) > 0 {
		fmt.Fprintf(&b, "parents = %s\n", quoteList(meta.Parents))
	}
	b.WriteString("+++\n")
	if !strings.HasPrefix(body, "\n") {
		b.WriteByte('\n')
	}
	b.WriteString(body)
	if !strings.HasSuffix(body, "\n") {
		b.WriteByte('\n')
	}
	return []byte(b.String())
}

func managed(existing, block string) string {
	section := managedStart + "\n" + strings.TrimSpace(block) + "\n" + managedEnd
	start := strings.Index(existing, managedStart)
	end := strings.Index(existing, managedEnd)
	if start >= 0 && end >= start {
		return strings.TrimRight(existing[:start], "\n") + "\n\n" + section + strings.TrimLeft(existing[end+len(managedEnd):], "\n")
	}
	if strings.TrimSpace(existing) == "" {
		return section + "\n"
	}
	return strings.TrimRight(existing, "\n") + "\n\n" + section + "\n"
}
