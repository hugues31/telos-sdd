package telos

import (
	"bufio"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"io/fs"
	"os"
	"path"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

func findRoot(start string) (string, error) {
	p, err := filepath.Abs(start)
	if err != nil {
		return "", err
	}
	for {
		if st, err := os.Stat(filepath.Join(p, configFile)); err == nil && st.Mode().IsRegular() {
			return p, nil
		}
		next := filepath.Dir(p)
		if next == p {
			return "", errors.New("not inside a Telos project; run `telos init`")
		}
		p = next
	}
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
	var originalMode fs.FileMode
	madeDestinationWritable := false
	if info, statErr := os.Lstat(path); statErr == nil && info.Mode().IsRegular() && info.Mode().Perm()&0o200 == 0 {
		originalMode = info.Mode().Perm()
		if err = os.Chmod(path, originalMode|0o200); err != nil {
			return err
		}
		madeDestinationWritable = true
	}
	if err = os.Rename(tmp, path); err != nil {
		if madeDestinationWritable {
			_ = os.Chmod(path, originalMode)
		}
		return err
	}
	return nil
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

func rootHashMap(files map[string]string) string {
	paths := make([]string, 0, len(files))
	for p := range files {
		paths = append(paths, p)
	}
	sort.Strings(paths)
	h := sha256.New()
	for _, p := range paths {
		io.WriteString(h, p)
		h.Write([]byte{0})
		io.WriteString(h, files[p])
		h.Write([]byte{'\n'})
	}
	return hex.EncodeToString(h.Sum(nil))
}

// globMatch matches slash-separated relative paths. `*` and `?` stay within
// one path segment; `**` spans any number of segments, including zero.
func globMatch(pattern, rel string) bool {
	return matchSegments(strings.Split(pattern, "/"), strings.Split(rel, "/"))
}

func matchSegments(pattern, segments []string) bool {
	if len(pattern) == 0 {
		return len(segments) == 0
	}
	if pattern[0] == "**" {
		if matchSegments(pattern[1:], segments) {
			return true
		}
		return len(segments) > 0 && matchSegments(pattern, segments[1:])
	}
	if len(segments) == 0 {
		return false
	}
	if ok, err := path.Match(pattern[0], segments[0]); err != nil || !ok {
		return false
	}
	return matchSegments(pattern[1:], segments[1:])
}

func matchAny(patterns []string, rel string) bool {
	for _, pattern := range patterns {
		if globMatch(pattern, rel) {
			return true
		}
	}
	return false
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

func quoteList(v []string) string {
	parts := make([]string, len(v))
	for i, s := range v {
		parts[i] = strconv.Quote(s)
	}
	return "[" + strings.Join(parts, ", ") + "]"
}

func stripComment(line string) string {
	return strings.SplitN(line, "#", 2)[0]
}

func readConfig(root string) (Config, error) {
	cfg := Config{}
	f, err := os.Open(filepath.Join(root, configFile))
	if err != nil {
		return cfg, coded("TELOS_CONFIG_INVALID", "telos.toml is missing or unreadable; run `telos init`")
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
		case "agents":
			cfg.Agents = parseList(val)
		case "test_commands":
			cfg.TestCommands = parseList(val)
		case "test_files":
			cfg.TestFiles = parseList(val)
		case "untraced":
			cfg.Untraced = parseList(val)
		default:
			return cfg, coded("TELOS_CONFIG_INVALID", "unknown key "+strconv.Quote(key)+" in telos.toml; valid keys: agents, test_commands, test_files, untraced")
		}
	}
	if err := s.Err(); err != nil {
		return cfg, coded("TELOS_CONFIG_INVALID", "telos.toml is unreadable: "+err.Error())
	}
	return cfg, nil
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
