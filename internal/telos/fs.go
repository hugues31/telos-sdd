package telos

import (
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/hugues31/telos-sdd/internal/kernel"
)

const (
	managedStart = "<!-- telos:managed:start -->"
	managedEnd   = "<!-- telos:managed:end -->"
)

// findRoot walks up from start to the directory holding telos.toml. In V2 the
// configuration lives at the repository root (tracked, protected), so this
// finds the project in the certified worktree and in candidate worktrees
// alike.
func findRoot(start string) (string, error) {
	p, err := filepath.Abs(start)
	if err != nil {
		return "", err
	}
	for {
		if st, err := os.Stat(filepath.Join(p, kernel.ConfigFile)); err == nil && st.Mode().IsRegular() {
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

func normalize(data []byte) []byte {
	s := strings.ReplaceAll(string(data), "\r\n", "\n")
	s = strings.ReplaceAll(s, "\r", "\n")
	return []byte(s)
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
