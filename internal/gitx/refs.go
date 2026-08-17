package gitx

import (
	"fmt"
	"os"
	"os/exec"
	"sort"
	"strings"
)

// RefUpdate is one operation of an atomic ref transaction. Old is the
// compare-and-swap expectation; Create asserts the ref does not exist yet.
type RefUpdate struct {
	Ref    string
	New    OID
	Old    OID
	Create bool
}

// RefTransaction applies all updates atomically via `git update-ref --stdin`:
// either every ref moves, or none does. A failed compare-and-swap fails the
// whole transaction — this is the kernel's last line of defense against
// concurrent promotions (KERNEL-001 under races).
func (r *Repo) RefTransaction(updates []RefUpdate) error {
	var lines strings.Builder
	lines.WriteString("start\n")
	for _, u := range updates {
		if u.Create {
			fmt.Fprintf(&lines, "create %s %s\n", u.Ref, u.New)
		} else {
			fmt.Fprintf(&lines, "update %s %s %s\n", u.Ref, u.New, u.Old)
		}
	}
	lines.WriteString("prepare\ncommit\n")
	_, err := r.run([]byte(lines.String()), "update-ref", "--stdin")
	return err
}

// TreeEntry is one blob entry of a tree, with its mode preserved.
type TreeEntry struct {
	Mode string
	Path string
	OID  OID
}

// LsTreeEntries lists every blob of a revision's tree with modes.
func (r *Repo) LsTreeEntries(rev string) ([]TreeEntry, error) {
	out, err := r.run(nil, "ls-tree", "-r", "-z", "--full-tree", rev)
	if err != nil {
		return nil, err
	}
	var entries []TreeEntry
	for _, rec := range strings.Split(out, "\x00") {
		meta, path, ok := strings.Cut(rec, "\t")
		if !ok {
			continue
		}
		fields := strings.Fields(meta)
		if len(fields) != 3 || fields[1] != "blob" {
			continue
		}
		entries = append(entries, TreeEntry{Mode: fields[0], Path: path, OID: OID(fields[2])})
	}
	return entries, nil
}

// TreeFromTreeEntries writes a tree from mode-preserving entries using a
// temporary index.
func (r *Repo) TreeFromTreeEntries(entries []TreeEntry) (OID, error) {
	sorted := append([]TreeEntry(nil), entries...)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i].Path < sorted[j].Path })

	var lines strings.Builder
	for _, e := range sorted {
		fmt.Fprintf(&lines, "%s %s\t%s\n", e.Mode, e.OID, e.Path)
	}
	tmp, err := os.CreateTemp("", "telos-index-*")
	if err != nil {
		return "", err
	}
	tmpPath := tmp.Name()
	tmp.Close()
	os.Remove(tmpPath)
	defer os.Remove(tmpPath)

	env := append(os.Environ(), "GIT_TERMINAL_PROMPT=0", "GIT_INDEX_FILE="+tmpPath)
	index := exec.Command("git", "update-index", "--add", "--index-info")
	index.Dir = r.WorkDir
	index.Env = env
	index.Stdin = strings.NewReader(lines.String())
	if out, err := index.CombinedOutput(); err != nil {
		return "", fmt.Errorf("git update-index: %w: %s", err, strings.TrimSpace(string(out)))
	}
	write := exec.Command("git", "write-tree")
	write.Dir = r.WorkDir
	write.Env = env
	out, err := write.Output()
	if err != nil {
		return "", fmt.Errorf("git write-tree: %w", err)
	}
	return OID(strings.TrimSpace(string(out))), nil
}

// NotesAddEntry builds a new notes commit carrying the note for the given
// commit (flat full-hex entry, replacing any fanned-out duplicate) WITHOUT
// moving the notes ref — the caller moves it inside a RefTransaction so the
// certified branch and its certificate advance together.
func (r *Repo) NotesAddEntry(notesRef string, commit OID, note []byte) (newNotes, oldNotes OID, err error) {
	blob, err := r.HashObject(note)
	if err != nil {
		return "", "", err
	}
	hex := string(commit)
	var entries []TreeEntry
	oldNotes, refErr := r.RevParse(notesRef)
	if refErr == nil {
		existing, err := r.LsTreeEntries(notesRef)
		if err != nil {
			return "", "", err
		}
		for _, e := range existing {
			if strings.ReplaceAll(e.Path, "/", "") == hex {
				continue // replaced by the new entry
			}
			entries = append(entries, e)
		}
	} else {
		oldNotes = ""
	}
	entries = append(entries, TreeEntry{Mode: "100644", Path: hex, OID: blob})
	tree, err := r.TreeFromTreeEntries(entries)
	if err != nil {
		return "", "", err
	}
	var parents []OID
	if oldNotes != "" {
		parents = append(parents, oldNotes)
	}
	newNotes, err = r.CommitTree(tree, parents, "telos: certificate for "+short(commit))
	return newNotes, oldNotes, err
}

func short(oid OID) string {
	if len(oid) > 12 {
		return string(oid[:12])
	}
	return string(oid)
}
