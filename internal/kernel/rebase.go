package kernel

import (
	"os"
	"path/filepath"
	"sort"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/evidence"
	"github.com/hugues31/telos-sdd/internal/gitx"
)

// RebaseReport is the outcome of rebasing a candidate onto the moved base:
// which proofs survived (closure unchanged — the concrete payoff of
// content-addressed evidence) and whether the approval carried.
type RebaseReport struct {
	ID                  string   `json:"id"`
	OldBase             string   `json:"old_base"`
	NewBase             string   `json:"new_base"`
	EvidenceKept        []string `json:"evidence_kept"`
	EvidenceInvalidated []string `json:"evidence_invalidated"`
	ApprovalsKept       bool     `json:"approvals_kept"`
}

// RebaseChange replays the candidate onto the current tip of its target
// branch, then selectively revalidates: evidence whose dependency closure
// digest is unchanged on the rebased tree survives; the rest is dropped and
// must be re-proven. The approval survives iff the recomputed digest equals
// the approved one (someone else touched spec/ ⇒ the human re-approves).
func RebaseChange(wt *gitx.Repo, cfg Config) (RebaseReport, error) {
	var report RebaseReport
	doc, err := LoadChange(wt)
	if err != nil {
		return report, err
	}
	if doc.Status == ChangePromoted {
		return report, coded.New("TELOS_CHANGE_STATE_INVALID", doc.ID+" is already promoted")
	}
	report.ID = doc.ID
	report.OldBase = doc.Base

	tip, err := wt.RevParse(doc.TargetBranch)
	if err != nil {
		return report, err
	}
	if string(tip) == doc.Base {
		return report, coded.New("TELOS_NOTHING_PENDING", "the base is current; nothing to rebase")
	}
	if _, err := wt.CommitAll("telos: snapshot " + doc.ID); err != nil {
		return report, err
	}
	if err := wt.RebaseOnto(tip, gitx.OID(doc.Base), doc.Branch); err != nil {
		return report, coded.New("TELOS_WORKTREE_CONFLICT", "the rebase conflicts with the new base; the candidate was restored — resolve by rebasing manually in the worktree, then rerun ("+err.Error()+")")
	}

	// The rebase rewrote the worktree; reload the change record from it.
	doc, err = LoadChange(wt)
	if err != nil {
		return report, err
	}
	doc.Base = string(tip)
	report.NewBase = string(tip)

	// Selective revalidation of this change's evidence.
	tree, err := wt.TreeOf("HEAD")
	if err != nil {
		return report, err
	}
	records, err := loadRecordsAt(wt, "HEAD")
	if err != nil {
		return report, err
	}
	for _, r := range records {
		if r.Change != doc.ID {
			continue
		}
		path := filepath.Join(wt.WorkDir, filepath.FromSlash(changeDir(doc.ID)), "evidence", evidence.FileName(r.Key()))
		digest, derr := evidence.Recompute(wt, tree, wt.WorkDir, r)
		if derr != nil || digest != r.DependsOn.ClosureDigest {
			if err := os.Remove(path); err != nil && !os.IsNotExist(err) {
				return report, err
			}
			report.EvidenceInvalidated = append(report.EvidenceInvalidated, r.ID)
			continue
		}
		r.SurvivedRebase = true
		if err := writeRecord(wt, doc, r); err != nil {
			return report, err
		}
		report.EvidenceKept = append(report.EvidenceKept, r.ID)
	}
	sort.Strings(report.EvidenceKept)
	sort.Strings(report.EvidenceInvalidated)

	// Approval survival is digest-bound: recompute on the new base.
	if err := saveChange(wt, doc); err != nil {
		return report, err
	}
	bundle, err := reviewComputation(wt, doc)
	if err == nil {
		for _, a := range doc.Approvals {
			if a.Kind == bundle.Kind && a.Digest == bundle.Digest {
				report.ApprovalsKept = true
				break
			}
		}
	}
	if !report.ApprovalsKept {
		doc.Approvals = []Approval{}
		doc.Review = nil
		if doc.Status == ChangeApproved || doc.Status == ChangeReady || doc.Status == ChangeAwaitingApproval {
			doc.Status = ChangeDrafting
		}
	}
	if err := saveChange(wt, doc); err != nil {
		return report, err
	}
	if _, err := wt.CommitAll("telos: rebase " + doc.ID + " onto " + short(tip)); err != nil {
		return report, err
	}
	return report, nil
}
