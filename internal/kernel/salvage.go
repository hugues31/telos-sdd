package kernel

import (
	"strings"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/gitx"
)

// SalvageResult names where the captured work went — the user's editor
// buffers point at reverted files, so the relocation must be explicit.
type SalvageResult struct {
	Change      string   `json:"change"`
	Worktree    string   `json:"worktree"`
	Root        string   `json:"root"`
	Paths       []string `json:"paths"`
	SpecTouched []string `json:"spec_touched,omitempty"`
}

// Salvage converts an out-of-band diff in the certified root into a Change:
// stash the diff, restore the certified bytes, create (or reuse) a candidate,
// apply the diff there. Salvage is the one-gesture normal path out of
// corruption — it moves work, it never destroys it (KERNEL-003).
func Salvage(repo *gitx.Repo, into, title string) (SalvageResult, error) {
	var result SalvageResult
	if ctx, err := ChangeContext(repo); err != nil {
		return result, err
	} else if ctx != "" {
		return result, coded.New("TELOS_ROOT_REQUIRED", "salvage runs in the certified root")
	}
	dirty, err := repo.DirtyPaths()
	if err != nil {
		return result, err
	}
	if len(dirty) == 0 {
		return result, coded.New("TELOS_NOTHING_PENDING", "the worktree matches the certified state; nothing to salvage")
	}
	head, err := repo.Head()
	if err != nil {
		return result, err
	}
	cert, err := LoadCertificate(repo, head)
	if err == nil {
		tree, terr := repo.TreeOf("HEAD")
		if terr != nil {
			return result, terr
		}
		err = cert.Validate(head, tree)
	}
	if err != nil {
		return result, coded.New("TELOS_CERTIFICATE_INVALID", "the tip itself is uncertified (out-of-band commit); salvage captures worktree diffs only — recover the commits with git first")
	}
	result.Paths = dirty
	for _, p := range dirty {
		if p == contract.Dir || strings.HasPrefix(p, contract.Dir+"/") {
			result.SpecTouched = append(result.SpecTouched, p)
		}
	}

	if err := repo.StashPush("telos salvage"); err != nil {
		return result, err
	}
	unstash := func() {
		_ = repo.StashApply()
		_ = repo.StashDrop()
	}

	var worktree string
	var changeID string
	if into != "" {
		changes, err := OpenChanges(repo)
		if err != nil {
			unstash()
			return result, err
		}
		for _, c := range changes {
			if c.ID == into {
				worktree, changeID = c.Worktree, c.ID
			}
		}
		if worktree == "" {
			unstash()
			return result, coded.New("TELOS_CHANGE_UNKNOWN", "no open change "+into+" with a worktree")
		}
	} else {
		if title == "" {
			title = "salvaged edits"
		}
		doc, path, err := StartChange(repo, CategoryBehaviorPreserving, title)
		if err != nil {
			unstash()
			return result, err
		}
		worktree, changeID = path, doc.ID
	}

	wt, err := gitx.Open(worktree)
	if err != nil {
		unstash()
		return result, err
	}
	if err := wt.StashApply(); err != nil {
		// The work is preserved: the stash survives, and whatever applied
		// cleanly sits in the candidate.
		return result, coded.WithPaths("TELOS_WORKTREE_CONFLICT",
			"the salvaged diff conflicts with "+changeID+"; your work is preserved in the git stash and partially applied in "+worktree, dirty)
	}
	if err := repo.StashDrop(); err != nil {
		return result, err
	}
	if _, err := wt.CommitAll("telos: salvage into " + changeID); err != nil {
		return result, err
	}
	result.Change = changeID
	result.Worktree = worktree
	result.Root = "restored to the certified state"
	return result, nil
}

// Restore discards the out-of-band diff, returning the certified bytes.
// Destructive and guard-gated.
func Restore(repo *gitx.Repo) ([]string, error) {
	if ctx, err := ChangeContext(repo); err != nil {
		return nil, err
	} else if ctx != "" {
		return nil, coded.New("TELOS_ROOT_REQUIRED", "restore runs in the certified root")
	}
	dirty, err := repo.DirtyPaths()
	if err != nil {
		return nil, err
	}
	if len(dirty) == 0 {
		return nil, coded.New("TELOS_NOTHING_PENDING", "the worktree already matches the certified state")
	}
	if err := repo.ResetHardTo("HEAD"); err != nil {
		return nil, err
	}
	if err := repo.CleanUntracked(); err != nil {
		return nil, err
	}
	return dirty, nil
}
