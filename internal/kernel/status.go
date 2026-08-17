package kernel

import (
	"errors"
	"os"
	"path/filepath"

	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/gitx"
)

// Project states.
const (
	StateCertified     = "certified"
	StateCorrupted     = "corrupted"
	StateUninitialized = "uninitialized"
)

// CertStatus summarizes the certificate of the current HEAD.
type CertStatus struct {
	Commit   string `json:"commit"`
	Change   string `json:"change"`
	SealedAt string `json:"sealed_at"`
}

// DirtyInfo lists the paths diverging from the certified state.
type DirtyInfo struct {
	Paths []string `json:"paths"`
}

// ContractCounts summarizes the certified contract.
type ContractCounts struct {
	Intents      int `json:"intents"`
	Requirements int `json:"requirements"`
	Decisions    int `json:"decisions"`
}

// ProjectStatus is the frozen result schema of `telos status` (root context;
// the candidate context and the salvage proposal arrive with M2/M4).
type ProjectStatus struct {
	Context     string          `json:"context"`
	State       string          `json:"state"`
	Certificate *CertStatus     `json:"certificate,omitempty"`
	Dirty       *DirtyInfo      `json:"dirty,omitempty"`
	Reason      string          `json:"reason,omitempty"`
	Contract    *ContractCounts `json:"contract,omitempty"`
}

// Status derives the project state from the substrate. It never fails on
// business states — corruption is a status, not an error.
func Status(repo *gitx.Repo) (ProjectStatus, error) {
	st := ProjectStatus{Context: "root"}
	if _, err := os.Stat(filepath.Join(repo.WorkDir, ConfigFile)); err != nil {
		st.State = StateUninitialized
		return st, nil
	}
	head, err := repo.Head()
	if errors.Is(err, gitx.ErrNoCommits) {
		st.State = StateUninitialized
		st.Reason = "no commits; run `telos init`"
		return st, nil
	} else if err != nil {
		return st, err
	}

	dirty, err := repo.DirtyPaths()
	if err != nil {
		return st, err
	}

	cert, err := LoadCertificate(repo, head)
	certValid := err == nil
	if certValid {
		tree, terr := repo.TreeOf("HEAD")
		if terr != nil {
			return st, terr
		}
		certValid = cert.Validate(head, tree) == nil
	}

	switch {
	case !certValid:
		st.State = StateCorrupted
		st.Reason = "HEAD carries no valid certificate (out-of-band commit?)"
		if len(dirty) > 0 {
			st.Dirty = &DirtyInfo{Paths: dirty}
		}
	case len(dirty) > 0:
		st.State = StateCorrupted
		st.Reason = "worktree diverged from the certified state"
		st.Dirty = &DirtyInfo{Paths: dirty}
	default:
		st.State = StateCertified
		st.Certificate = &CertStatus{Commit: string(head), Change: cert.Payload.Change.ID, SealedAt: cert.Payload.SealedAt}
		if files, ferr := contractFilesAt(repo, "HEAD"); ferr == nil {
			if parsed, problems := contract.Parse(files); len(problems) == 0 {
				st.Contract = &ContractCounts{Intents: len(parsed.Intents), Requirements: len(parsed.Requirements), Decisions: len(parsed.Decisions)}
			}
		}
	}
	return st, nil
}
