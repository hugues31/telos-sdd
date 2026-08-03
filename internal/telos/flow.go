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

func flowPath(root, id string) string {
	return filepath.Join(root, ".telos", "flows", strings.ToLower(id)+".json")
}

func loadFlow(root, id string) (Flow, error) {
	var flow Flow
	if err := readJSON(flowPath(root, id), &flow); err != nil {
		return flow, err
	}
	if flow.DraftHashes == nil {
		flow.DraftHashes = map[string]string{}
	}
	return flow, nil
}

func saveFlow(root string, flow Flow) error {
	flow.Updated = time.Now().UTC().Format(time.RFC3339)
	if flow.Created == "" {
		flow.Created = flow.Updated
	}
	if flow.DraftHashes == nil {
		flow.DraftHashes = map[string]string{}
	}
	return writeJSON(flowPath(root, flow.ID), flow)
}

func activeFlow(root string) (Flow, error) {
	paths, err := filepath.Glob(filepath.Join(root, ".telos", "flows", "*.json"))
	if err != nil {
		return Flow{}, err
	}
	var active []Flow
	for _, path := range paths {
		var flow Flow
		if err := readJSON(path, &flow); err != nil {
			return Flow{}, err
		}
		if flow.Status == "active" {
			active = append(active, flow)
		}
	}
	if len(active) == 0 {
		return Flow{}, os.ErrNotExist
	}
	if len(active) > 1 {
		return Flow{}, coded("TELOS_FLOW_AMBIGUOUS", "multiple active flows exist in this worktree")
	}
	if active[0].DraftHashes == nil {
		active[0].DraftHashes = map[string]string{}
	}
	return active[0], nil
}

func startFlow(root, request, brainstorm string) (Flow, error) {
	if strings.TrimSpace(request) == "" {
		return Flow{}, coded("TELOS_INPUT_REQUIRED", "flow request is required")
	}
	if flow, err := activeFlow(root); err == nil {
		return Flow{}, coded("TELOS_FLOW_ACTIVE", fmt.Sprintf("flow %s is already active", flow.ID))
	} else if !errors.Is(err, os.ErrNotExist) {
		return Flow{}, err
	}
	id, err := newID("flw", time.Now())
	if err != nil {
		return Flow{}, err
	}
	flow := Flow{ID: id, Status: "active", Phase: "discovery", Request: strings.TrimSpace(request), DraftHashes: map[string]string{}}
	if brainstorm == "" {
		brainstorm = "recommend"
	}
	if brainstorm == "none" {
		intentID, path, err := newIntent(root, request, "")
		if err != nil {
			return Flow{}, err
		}
		if err := bindArtifactToFlow(filepath.Join(root, filepath.FromSlash(path)), id); err != nil {
			return Flow{}, err
		}
		flow.Intent = intentID
		flow.Phase = "intent_draft"
		artifactPath := filepath.Join(root, filepath.FromSlash(path))
		h, _ := fileHash(artifactPath)
		if err := storeBlob(root, artifactPath, h); err != nil {
			return Flow{}, err
		}
		flow.DraftHashes[intentID] = h
	} else {
		brainstormID, path, err := startBrainstorm(root, brainstorm, 0)
		if err != nil {
			return Flow{}, err
		}
		if err := bindArtifactToFlow(filepath.Join(root, filepath.FromSlash(path)), id); err != nil {
			return Flow{}, err
		}
		flow.Brainstorm = brainstormID
		flow.Phase = "brainstorming"
		artifactPath := filepath.Join(root, filepath.FromSlash(path))
		h, _ := fileHash(artifactPath)
		if err := storeBlob(root, artifactPath, h); err != nil {
			return Flow{}, err
		}
		flow.DraftHashes[brainstormID] = h
	}
	if err := saveFlow(root, flow); err != nil {
		return Flow{}, err
	}
	if err := appendEvent(root, "flow.started", flow.ID, map[string]any{"request": flow.Request, "brainstorm": brainstorm}, ""); err != nil {
		return Flow{}, err
	}
	return flow, nil
}

func bindArtifactToFlow(path, flowID string) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	meta, body, err := parseArtifact(data)
	if err != nil {
		return err
	}
	meta.Flow = flowID
	return atomicWrite(path, renderArtifact(meta, body), 0o644)
}

func artifactKind(id string) (string, error) {
	switch {
	case strings.HasPrefix(strings.ToUpper(id), "BRN-"):
		return "brainstorm", nil
	case strings.HasPrefix(strings.ToUpper(id), "INT-"):
		return "intent", nil
	case strings.HasPrefix(strings.ToUpper(id), "SPC-"):
		return "spec", nil
	default:
		return "", fmt.Errorf("unsupported artifact id %q", id)
	}
}

func putArtifact(root, id, body string) (string, error) {
	kind, err := artifactKind(id)
	if err != nil {
		return "", err
	}
	path, meta, _, err := findArtifact(root, kind, id)
	if err != nil {
		return "", err
	}
	if meta.Status == "sealed" {
		return "", coded("TELOS_ARTIFACT_SEALED", "sealed artifacts cannot be changed")
	}
	if strings.TrimSpace(body) == "" {
		return "", coded("TELOS_INPUT_REQUIRED", "artifact body is required")
	}
	if strings.HasPrefix(strings.TrimSpace(body), "+++") {
		return "", coded("TELOS_INPUT_INVALID", "artifact put accepts the Markdown body without frontmatter")
	}
	meta.Revision++
	if err := atomicWrite(path, renderArtifact(meta, strings.TrimSpace(body)+"\n"), 0o644); err != nil {
		return "", err
	}
	h, err := fileHash(path)
	if err != nil {
		return "", err
	}
	if err := storeBlob(root, path, h); err != nil {
		return "", err
	}
	if meta.Flow != "" {
		flow, err := loadFlow(root, meta.Flow)
		if err != nil {
			return "", err
		}
		flow.DraftHashes[id] = h
		flow.IntentReview = ""
		flow.ContractReview = ""
		if err := saveFlow(root, flow); err != nil {
			return "", err
		}
	}
	if err := appendEvent(root, kind+".updated", id, map[string]any{"path": relative(root, path), "hash": h, "revision": meta.Revision}, ""); err != nil {
		return "", err
	}
	return relative(root, path), nil
}

func reviseArtifact(root, id, reason string) (Flow, string, string, error) {
	kind, err := artifactKind(id)
	if err != nil || kind == "brainstorm" {
		return Flow{}, "", "", coded("TELOS_INPUT_INVALID", "only sealed intents and specs can be revised")
	}
	_, meta, body, err := findArtifact(root, kind, id)
	if err != nil {
		return Flow{}, "", "", err
	}
	if meta.Status != "sealed" || meta.Flow == "" {
		return Flow{}, "", "", coded("TELOS_PHASE_INVALID", "artifact is not a sealed flow artifact")
	}
	flow, err := loadFlow(root, meta.Flow)
	if err != nil {
		return flow, "", "", err
	}
	if flow.Status != "active" {
		return flow, "", "", coded("TELOS_PHASE_INVALID", "completed flows cannot be revised; start a new flow")
	}
	if flow.Change != "" {
		change, err := resolveChange(root, flow.ID, "")
		if err != nil {
			return flow, "", "", err
		}
		if change.Status == "active" {
			if strings.TrimSpace(reason) == "" {
				return flow, "", "", coded("TELOS_INPUT_REQUIRED", "revision during implementation requires an abort reason")
			}
			if _, err := abortChange(root, change, reason); err != nil {
				return flow, "", "", err
			}
			flow.Change = ""
		}
	}
	prefix := "int"
	if kind == "spec" {
		prefix = "spc"
	}
	newIDValue, err := newID(prefix, time.Now())
	if err != nil {
		return flow, "", "", err
	}
	meta.ID = newIDValue
	meta.Status = "draft"
	meta.Revision++
	meta.Supersedes = id
	meta.Parents = append(meta.Parents, id)
	path := filepath.Join(root, ".telos", kind+"s", strings.ToLower(newIDValue)+".md")
	if err := atomicWrite(path, renderArtifact(meta, body), 0o644); err != nil {
		return flow, "", "", err
	}
	h, _ := fileHash(path)
	if err := storeBlob(root, path, h); err != nil {
		return flow, "", "", err
	}
	flow.DraftHashes[newIDValue] = h
	flow.IntentReview = ""
	flow.ContractReview = ""
	if kind == "intent" {
		flow.Intent = newIDValue
		flow.Specs = nil
		flow.Phase = "intent_draft"
	} else {
		for i, specID := range flow.Specs {
			if specID == id {
				flow.Specs[i] = newIDValue
			}
		}
		flow.Phase = "contract_draft"
	}
	if err := saveFlow(root, flow); err != nil {
		return flow, "", "", err
	}
	if err := appendEvent(root, kind+".revised", newIDValue, map[string]any{"supersedes": id, "reason": strings.TrimSpace(reason), "path": relative(root, path)}, ""); err != nil {
		return flow, "", "", err
	}
	return flow, newIDValue, relative(root, path), nil
}

func attachIntent(root, flowID, title string) (Flow, string, error) {
	flow, err := loadFlow(root, flowID)
	if err != nil {
		return flow, "", err
	}
	if flow.Status != "active" || flow.Intent != "" {
		return flow, "", coded("TELOS_PHASE_INVALID", "flow cannot accept a new intent")
	}
	from := flow.Brainstorm
	id, path, err := newIntent(root, title, from)
	if err != nil {
		return flow, "", err
	}
	abs := filepath.Join(root, filepath.FromSlash(path))
	if err := bindArtifactToFlow(abs, flow.ID); err != nil {
		return flow, "", err
	}
	flow.Intent = id
	flow.Phase = "intent_draft"
	h, _ := fileHash(abs)
	if err := storeBlob(root, abs, h); err != nil {
		return flow, "", err
	}
	flow.DraftHashes[id] = h
	if err := saveFlow(root, flow); err != nil {
		return flow, "", err
	}
	return flow, path, nil
}

func attachSpec(root, flowID, title string) (Flow, string, error) {
	flow, err := loadFlow(root, flowID)
	if err != nil {
		return flow, "", err
	}
	if flow.Intent == "" {
		return flow, "", coded("TELOS_PHASE_INVALID", "flow has no intent")
	}
	id, path, err := newSpec(root, flow.Intent, title)
	if err != nil {
		return flow, "", err
	}
	abs := filepath.Join(root, filepath.FromSlash(path))
	if err := bindArtifactToFlow(abs, flow.ID); err != nil {
		return flow, "", err
	}
	flow.Specs = append(flow.Specs, id)
	flow.Phase = "contract_draft"
	h, _ := fileHash(abs)
	if err := storeBlob(root, abs, h); err != nil {
		return flow, "", err
	}
	flow.DraftHashes[id] = h
	if err := saveFlow(root, flow); err != nil {
		return flow, "", err
	}
	return flow, path, nil
}

func reviewIntent(root, flowID string) (Flow, string, string, error) {
	flow, err := loadFlow(root, flowID)
	if err != nil {
		return flow, "", "", err
	}
	path, meta, body, err := findArtifact(root, "intent", flow.Intent)
	if err != nil {
		return flow, "", "", err
	}
	if meta.Status != "draft" {
		return flow, "", "", coded("TELOS_PHASE_INVALID", "intent is not a draft")
	}
	if err := validateBody("intent", body); err != nil {
		return flow, "", "", err
	}
	if len(criterionIDs(body)) == 0 {
		return flow, "", "", coded("TELOS_CONTRACT_INVALID", "intent must contain at least one CRIT-NNN heading")
	}
	digest, err := fileHash(path)
	if err != nil {
		return flow, "", "", err
	}
	flow.IntentReview = digest
	flow.Phase = "intent_review"
	if err := saveFlow(root, flow); err != nil {
		return flow, "", "", err
	}
	return flow, digest, body, nil
}

func sealReviewedIntent(root, flowID, digest string) (Flow, error) {
	flow, err := loadFlow(root, flowID)
	if err != nil {
		return flow, err
	}
	if digest == "" {
		return flow, coded("TELOS_APPROVAL_REQUIRED", "intent seal requires the current review digest")
	}
	if flow.IntentReview == "" || digest != flow.IntentReview {
		return flow, coded("TELOS_APPROVAL_STALE", "intent review is missing or stale")
	}
	path, _, _, err := findArtifact(root, "intent", flow.Intent)
	if err != nil {
		return flow, err
	}
	current, err := fileHash(path)
	if err != nil {
		return flow, err
	}
	if current != digest {
		return flow, coded("TELOS_APPROVAL_STALE", "intent changed after it was reviewed")
	}
	if err := sealArtifact(root, "intent", flow.Intent); err != nil {
		return flow, err
	}
	flow.Phase = "contract_draft"
	h, _ := fileHash(path)
	if err := storeBlob(root, path, h); err != nil {
		return flow, err
	}
	flow.DraftHashes[flow.Intent] = h
	if err := saveFlow(root, flow); err != nil {
		return flow, err
	}
	return flow, nil
}

func auditFlowDrafts(root string, flow Flow) error {
	ids := make([]string, 0, len(flow.DraftHashes))
	for id := range flow.DraftHashes {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	for _, id := range ids {
		if strings.HasSuffix(id, ":plan") {
			specID := strings.TrimSuffix(id, ":plan")
			path := filepath.Join(root, ".telos", "test-plans", strings.ToLower(specID)+".json")
			h, err := fileHash(path)
			if err != nil || h != flow.DraftHashes[id] {
				return codedPaths("TELOS_INTEGRITY_UNDECLARED_CHANGE", "project corrupted: test plan changed outside the Telos CLI", []string{relative(root, path)})
			}
			continue
		}
		kind, err := artifactKind(id)
		if err != nil {
			return err
		}
		path, _, _, err := findArtifact(root, kind, id)
		if err != nil {
			return codedPaths("TELOS_INTEGRITY_UNDECLARED_CHANGE", "project corrupted: managed artifact is missing", []string{id})
		}
		h, err := fileHash(path)
		if err != nil {
			return err
		}
		if h != flow.DraftHashes[id] {
			return codedPaths("TELOS_INTEGRITY_UNDECLARED_CHANGE", "project corrupted: artifact changed outside the Telos CLI", []string{relative(root, path)})
		}
	}
	return nil
}
