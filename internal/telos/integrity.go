package telos

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

func loadLock(root string) (Lock, error) {
	var lock Lock
	err := readJSON(filepath.Join(root, ".telos", "lock.json"), &lock)
	if os.IsNotExist(err) {
		return Lock{Artifacts: []LockedFile{}}, nil
	}
	return lock, err
}

func saveLock(root string, lock Lock) error {
	sort.Slice(lock.Artifacts, func(i, j int) bool { return lock.Artifacts[i].Path < lock.Artifacts[j].Path })
	lock.RootHash = rootHash(lock.Artifacts)
	return writeJSON(filepath.Join(root, ".telos", "lock.json"), lock)
}

func lockFile(root string, entry LockedFile) (Lock, error) {
	lock, err := loadLock(root)
	if err != nil {
		return lock, err
	}
	found := false
	for i := range lock.Artifacts {
		if lock.Artifacts[i].Path == entry.Path {
			lock.Artifacts[i] = entry
			found = true
		}
	}
	if !found {
		lock.Artifacts = append(lock.Artifacts, entry)
	}
	lock.RootHash = rootHash(lock.Artifacts)
	err = saveLock(root, lock)
	return lock, err
}

func appendEvent(root, typ, subject string, data map[string]any, hash string) error {
	id, err := newID("evt", time.Now())
	if err != nil {
		return err
	}
	e := Event{ID: id, At: time.Now().UTC(), Type: typ, Subject: subject, Data: data, RootHash: hash}
	if err := writeJSON(filepath.Join(root, ".telos", "ledger", "events", strings.ToLower(id)+".json"), e); err != nil {
		return err
	}
	return rebuildState(root)
}

func rebuildState(root string) error {
	files, err := filepath.Glob(filepath.Join(root, ".telos", "ledger", "events", "*.json"))
	if err != nil {
		return err
	}
	events := make([]Event, 0, len(files))
	for _, path := range files {
		var e Event
		if err := readJSON(path, &e); err != nil {
			return fmt.Errorf("read event %s: %w", filepath.Base(path), err)
		}
		events = append(events, e)
	}
	sort.Slice(events, func(i, j int) bool {
		if events[i].At.Equal(events[j].At) {
			return events[i].ID < events[j].ID
		}
		return events[i].At.Before(events[j].At)
	})
	state := State{Status: map[string]string{}}
	for _, e := range events {
		state.Events++
		state.LatestEvent = e.ID
		if e.RootHash != "" {
			state.RootHash = e.RootHash
		}
		if e.Subject != "" {
			state.Status[e.Subject] = e.Type
		}
	}
	return writeJSON(filepath.Join(root, ".telos", "state.json"), state)
}

type AuditResult struct {
	Path   string
	Status string
	Detail string
}

func audit(root string) ([]AuditResult, error) {
	lock, err := loadLock(root)
	if err != nil {
		return nil, err
	}
	var out []AuditResult
	badIDs := map[string]bool{}
	actualEntries := make([]LockedFile, 0, len(lock.Artifacts))
	for _, f := range lock.Artifacts {
		path := filepath.Join(root, filepath.FromSlash(f.Path))
		h, err := fileHash(path)
		entry := f
		if os.IsNotExist(err) {
			out = append(out, AuditResult{f.Path, "missing", "sealed artifact was removed"})
			badIDs[f.ID] = true
			continue
		}
		if err != nil {
			return nil, err
		}
		entry.Hash = h
		actualEntries = append(actualEntries, entry)
		if h != f.Hash {
			out = append(out, AuditResult{f.Path, "tampered", "content differs from sealed hash"})
			badIDs[f.ID] = true
			continue
		}
		out = append(out, AuditResult{f.Path, "ok", ""})
	}
	for changed := true; changed; {
		changed = false
		for _, f := range lock.Artifacts {
			if badIDs[f.ID] {
				continue
			}
			for _, parent := range f.Parents {
				missingRequired := (strings.HasPrefix(parent, "INT-") || strings.HasPrefix(parent, "SPC-")) && !artifactIDInLock(lock, parent)
				if badIDs[parent] || missingRequired {
					badIDs[f.ID], changed = true, true
					for j := range out {
						if out[j].Path == f.Path {
							out[j] = AuditResult{f.Path, "stale", "a sealed parent is missing or invalid"}
						}
					}
					break
				}
			}
		}
	}
	contentMismatch := len(actualEntries) == len(lock.Artifacts) && rootHash(actualEntries) != lock.RootHash
	ledgerMismatch := false
	ledgerRoot, err := latestLedgerRoot(root)
	if err != nil {
		return nil, err
	}
	if (len(lock.Artifacts) > 0 && ledgerRoot == "") || (ledgerRoot != "" && ledgerRoot != lock.RootHash) {
		ledgerMismatch = true
	}
	if contentMismatch || ledgerMismatch {
		detail := "root hash differs from locked content"
		if ledgerMismatch {
			detail = "root hash differs from ledger evidence"
		}
		out = append(out, AuditResult{".telos/lock.json", "tampered", detail})
	}
	return out, nil
}

func latestLedgerRoot(root string) (string, error) {
	files, err := filepath.Glob(filepath.Join(root, ".telos", "ledger", "events", "*.json"))
	if err != nil {
		return "", err
	}
	events := make([]Event, 0, len(files))
	for _, path := range files {
		var event Event
		if err := readJSON(path, &event); err != nil {
			return "", fmt.Errorf("read event %s: %w", filepath.Base(path), err)
		}
		events = append(events, event)
	}
	sort.Slice(events, func(i, j int) bool {
		if events[i].At.Equal(events[j].At) {
			return events[i].ID < events[j].ID
		}
		return events[i].At.Before(events[j].At)
	})
	latest := ""
	for _, event := range events {
		if event.RootHash != "" {
			latest = event.RootHash
		}
	}
	return latest, nil
}

func artifactIDInLock(lock Lock, id string) bool {
	for _, f := range lock.Artifacts {
		if f.ID == id {
			return true
		}
	}
	return false
}

func requireCleanAudit(root string) error {
	results, err := audit(root)
	if err != nil {
		return err
	}
	var bad []string
	for _, r := range results {
		if r.Status != "ok" {
			detail := r.Status
			if r.Detail != "" {
				detail += ": " + r.Detail
			}
			bad = append(bad, r.Path+" ("+detail+")")
		}
	}
	if len(bad) > 0 {
		return errors.New("integrity check failed: " + strings.Join(bad, ", "))
	}
	return nil
}

func findArtifact(root, kind, id string) (string, ArtifactMeta, string, error) {
	dir := kind + "s"
	if kind == "spec" {
		dir = "specs"
	}
	paths, err := filepath.Glob(filepath.Join(root, ".telos", dir, "*.md"))
	if err != nil {
		return "", ArtifactMeta{}, "", err
	}
	for _, path := range paths {
		data, err := os.ReadFile(path)
		if err != nil {
			return "", ArtifactMeta{}, "", err
		}
		meta, body, err := parseArtifact(data)
		if err == nil && strings.EqualFold(meta.ID, id) {
			return path, meta, body, nil
		}
	}
	return "", ArtifactMeta{}, "", fmt.Errorf("%s %s not found", kind, id)
}

func validateBody(kind, body string) error {
	lower := strings.ToLower(body)
	if strings.Contains(lower, "todo") || strings.Contains(lower, "tbd") || strings.Contains(lower, "à compléter") {
		return errors.New("unresolved placeholder (TODO/TBD) found")
	}
	required := map[string][]string{
		"intent": {"## Outcome", "## Actors", "## Scope", "## Non-goals", "## Success criteria", "## Constraints", "## Open questions"},
		"spec":   {"## Context", "## Rules", "## Examples", "## Boundaries", "## Non-effects", "## Failure modes", "## Observability"},
	}
	for _, heading := range required[kind] {
		if !strings.Contains(body, heading) {
			return fmt.Errorf("missing required heading %q", heading)
		}
	}
	if kind == "intent" {
		section := sectionText(body, "## Open questions")
		if strings.TrimSpace(section) != "None." {
			return errors.New("open questions must be resolved; write exactly `None.` when none remain")
		}
	}
	if kind == "spec" && !strings.Contains(body, "RULE-") {
		return errors.New("spec must contain at least one stable RULE-* identifier")
	}
	return nil
}

func sectionText(body, heading string) string {
	i := strings.Index(body, heading)
	if i < 0 {
		return ""
	}
	rest := body[i+len(heading):]
	if j := strings.Index(rest, "\n## "); j >= 0 {
		rest = rest[:j]
	}
	return strings.TrimSpace(rest)
}
