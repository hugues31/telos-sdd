package kernel

import (
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/gitx"
)

// ReviewBundle is what the orchestrator presents to the human: the exact
// digest the approval will bind to, and the content it covers.
type ReviewBundle struct {
	Digest      string   `json:"digest"`
	Kind        string   `json:"kind"`
	Category    string   `json:"category"`
	Delta       string   `json:"delta,omitempty"`
	SpecChanged []string `json:"spec_changed,omitempty"`
	CodePaths   []string `json:"code_paths,omitempty"`
	Privileged  bool     `json:"privileged"`
}

// reviewComputation is the shared digest computation of review and approve:
// snapshot the worktree, verify the candidate never touched spec/ directly,
// flag privileged content, and compute the digest for the change's category.
func reviewComputation(wt *gitx.Repo, doc *ChangeDoc) (ReviewBundle, error) {
	var bundle ReviewBundle
	bundle.Category = doc.Category

	if _, err := wt.CommitAll("telos: snapshot " + doc.ID); err != nil {
		return bundle, err
	}
	changed, err := wt.DiffNames(doc.Base, "HEAD")
	if err != nil {
		return bundle, err
	}
	var specTouched []string
	for _, path := range changed {
		if path == contract.Dir || strings.HasPrefix(path, contract.Dir+"/") {
			specTouched = append(specTouched, path)
		}
		if path == ConfigFile || strings.HasPrefix(path, "policies/") {
			bundle.Privileged = true
		}
		if !strings.HasPrefix(path, changeDir(doc.ID)+"/") {
			bundle.CodePaths = append(bundle.CodePaths, path)
		}
	}
	if len(specTouched) > 0 {
		return bundle, coded.WithPaths("TELOS_CONTRACT_TAMPERED", "spec/ was edited directly in the candidate; contract semantics go through contract.delta.md — revert the direct edit and use the delta", specTouched)
	}

	deltaPath := filepath.Join(wt.WorkDir, filepath.FromSlash(changeDir(doc.ID)), "contract.delta.md")
	deltaBytes, err := os.ReadFile(deltaPath)
	if err != nil {
		return bundle, coded.New("TELOS_CHANGE_UNKNOWN", doc.ID+" has no contract.delta.md")
	}
	ops, err := contract.ParseDelta(deltaBytes)
	if err != nil {
		return bundle, coded.New("TELOS_CONTRACT_INVALID", err.Error())
	}

	switch doc.Category {
	case CategoryBehaviorChange:
		if len(ops) == 0 {
			return bundle, coded.New("TELOS_NOTHING_PENDING", "a behavior change needs a non-empty contract delta; describe the target contract in contract.delta.md")
		}
		baseSpec, err := contractFilesAt(wt, doc.Base)
		if err != nil {
			return bundle, err
		}
		folded, err := contract.Fold(baseSpec, ops)
		if err != nil {
			return bundle, coded.New("TELOS_CONTRACT_INVALID", err.Error())
		}
		if _, problems := contract.Parse(folded); len(problems) > 0 {
			return bundle, coded.WithPaths("TELOS_CONTRACT_INVALID", "the folded target contract is structurally invalid", problems)
		}
		tree, err := wt.TreeFromFiles(folded)
		if err != nil {
			return bundle, err
		}
		specTree, err := wt.SubtreeOf(string(tree), contract.Dir)
		if err != nil || specTree == "" {
			return bundle, coded.New("TELOS_CONTRACT_INVALID", "the folded contract is empty")
		}
		bundle.Digest = string(specTree)
		bundle.Kind = "contract"
		bundle.Delta = string(deltaBytes)
		bundle.SpecChanged = foldedChanges(baseSpec, folded)
	case CategoryBehaviorPreserving:
		if len(ops) > 0 {
			return bundle, coded.New("TELOS_INPUT_INVALID", "a behavior-preserving change cannot carry a contract delta; use a behavior_change")
		}
		// The claim covers the candidate tree WITHOUT the change record
		// itself, so review/approve bookkeeping commits cannot invalidate
		// the digest they are about to bind.
		files, err := wt.LsTree("HEAD")
		if err != nil {
			return bundle, err
		}
		for path := range files {
			if strings.HasPrefix(path, changeDir(doc.ID)+"/") {
				delete(files, path)
			}
		}
		tree, err := wt.TreeFromEntries(files)
		if err != nil {
			return bundle, err
		}
		bundle.Digest = string(tree)
		bundle.Kind = "preserving_claim"
	default:
		return bundle, coded.New("TELOS_CHANGE_STATE_INVALID", "category "+doc.Category+" cannot be reviewed")
	}
	return bundle, nil
}

func foldedChanges(base, folded map[string][]byte) []string {
	var out []string
	for path, content := range folded {
		if prev, ok := base[path]; !ok || string(prev) != string(content) {
			out = append(out, path)
		}
	}
	for path := range base {
		if _, ok := folded[path]; !ok {
			out = append(out, path+" (removed)")
		}
	}
	sort.Strings(out)
	return out
}

// ReviewChange computes and records the pending review digest, returning the
// exact content to present to the human.
func ReviewChange(wt *gitx.Repo) (*ChangeDoc, ReviewBundle, error) {
	doc, err := LoadChange(wt)
	if err != nil {
		return nil, ReviewBundle{}, err
	}
	if doc.Status == ChangePromoted {
		return nil, ReviewBundle{}, coded.New("TELOS_CHANGE_STATE_INVALID", doc.ID+" is already promoted")
	}
	bundle, err := reviewComputation(wt, doc)
	if err != nil {
		return nil, ReviewBundle{}, err
	}
	doc.Review = &PendingReview{Kind: bundle.Kind, Digest: bundle.Digest}
	doc.Privileged = doc.Privileged || bundle.Privileged
	if doc.Status == ChangeDrafting || doc.Status == ChangeApproved {
		doc.Status = ChangeAwaitingApproval
	}
	if err := saveChange(wt, doc); err != nil {
		return nil, ReviewBundle{}, err
	}
	if _, err := wt.CommitAll("telos: review " + doc.ID); err != nil {
		return nil, ReviewBundle{}, err
	}
	return doc, bundle, nil
}

// ApproveChange records the digest-bound human approval (KERNEL-004): the
// presented digest must equal both the recorded review and its recomputation
// on the exact current candidate content.
func ApproveChange(wt *gitx.Repo, digest string) (*ChangeDoc, error) {
	doc, err := LoadChange(wt)
	if err != nil {
		return nil, err
	}
	if doc.Review == nil {
		return nil, coded.New("TELOS_NOTHING_PENDING", "nothing reviewed; run `telos change review` and present its content first")
	}
	bundle, err := reviewComputation(wt, doc)
	if err != nil {
		// The computation succeeded at review time, so any failure now means
		// the content drifted since — the approval is stale either way.
		if e, ok := coded.As(err); ok && (strings.HasPrefix(e.Code, "TELOS_CONTRACT") || e.Code == "TELOS_NOTHING_PENDING") {
			return nil, coded.New("TELOS_APPROVAL_STALE", "the reviewed content changed and no longer computes: "+err.Error())
		}
		return nil, err
	}
	if digest == "" || digest != doc.Review.Digest || digest != bundle.Digest {
		return nil, coded.New("TELOS_APPROVAL_STALE", "the digest does not match the reviewed content; run `telos change review` again and re-present")
	}
	doc.Approvals = append(doc.Approvals, Approval{Kind: bundle.Kind, Digest: digest, At: time.Now().UTC().Format(time.RFC3339)})
	doc.Review = nil
	doc.Status = ChangeApproved
	doc.Privileged = doc.Privileged || bundle.Privileged
	if err := saveChange(wt, doc); err != nil {
		return nil, err
	}
	if _, err := wt.CommitAll("telos: approve " + doc.ID); err != nil {
		return nil, err
	}
	return doc, nil
}
