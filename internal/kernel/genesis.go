package kernel

import (
	"runtime"
	"time"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/gitx"
	"github.com/hugues31/telos-sdd/internal/policy"
)

// GenesisOptions parameterizes the one sanctioned adoption of an existing
// tree into a certified state.
type GenesisOptions struct {
	// Version is the telos binary version recorded in the certificate.
	Version string
}

// Genesis adopts the current worktree as the initial certified state: stage
// everything, validate the contract on the staged tree (before any commit
// exists), commit (or reuse a clean HEAD), seal a genesis certificate onto
// the commit. It is explicit and guard-gated at the CLI; no other adoption
// path exists (KERNEL-003).
func Genesis(repo *gitx.Repo, cfg Config, opts GenesisOptions) (*Certificate, error) {
	if err := repo.AddAll(); err != nil {
		return nil, err
	}
	tree, err := repo.WriteTree()
	if err != nil {
		return nil, err
	}

	specFiles, err := contractFilesAt(repo, string(tree))
	if err != nil {
		return nil, err
	}
	parsed, problems := contract.Parse(specFiles)
	if len(problems) > 0 {
		return nil, coded.WithPaths("TELOS_CONTRACT_INVALID", "cannot certify an invalid contract", problems)
	}
	policyBlob, err := repo.RevParse(string(tree) + ":" + ConfigFile)
	if err != nil {
		return nil, coded.New("TELOS_NOT_INITIALIZED", "telos.toml is not part of the genesis tree")
	}
	eff, err := policy.Load(repo.WorkDir)
	if err != nil {
		return nil, err
	}
	specTree, err := repo.SubtreeOf(string(tree), contract.Dir)
	if err != nil {
		return nil, err
	}

	commit, err := repo.Head()
	switch {
	case err == nil:
		headTree, terr := repo.TreeOf("HEAD")
		if terr != nil {
			return nil, terr
		}
		if headTree != tree {
			commit, err = repo.CommitTree(tree, []gitx.OID{commit}, "telos: genesis")
			if err != nil {
				return nil, err
			}
			if err := advanceHead(repo, commit); err != nil {
				return nil, err
			}
		}
	default: // unborn HEAD
		commit, err = repo.CommitTree(tree, nil, "telos: genesis")
		if err != nil {
			return nil, err
		}
		if err := advanceHead(repo, commit); err != nil {
			return nil, err
		}
	}

	now := time.Now().UTC().Format(time.RFC3339)
	payload := CertPayload{
		Version:         1,
		Project:         ProjectInfo{ID: cfg.ProjectID, Genesis: string(commit)},
		Commit:          string(commit),
		Tree:            string(tree),
		ParentCertified: "",
		Change:          ChangeInfo{ID: "CHG-000", Category: CategoryGenesis, Base: ""},
		Contract:        ContractInfo{Tree: string(specTree), Requirements: sortedRequirementIDs(parsed), DeltaFrom: ""},
		Policy:          PolicyInfo{Blob: string(policyBlob), Hash: eff.Hash},
		Approvals:       []Approval{{Kind: "adoption", Digest: string(tree), At: now}},
		Verification:    Verification{Evidence: []EvidenceEntry{}, RequirementsVerified: []string{}, FindingsOpen: []string{}},
		Toolchain:       Toolchain{Telos: opts.Version, Go: runtime.Version()},
		SealedAt:        now,
	}
	return writeCertificate(repo, payload)
}

// advanceHead moves the branch HEAD points at (creating it on an unborn
// HEAD); on a detached HEAD it moves HEAD itself.
func advanceHead(repo *gitx.Repo, commit gitx.OID) error {
	ref, err := repo.HeadRef()
	if err != nil {
		return err
	}
	if ref == "" {
		ref = "HEAD"
	}
	if err := repo.UpdateRef(ref, commit); err != nil {
		return err
	}
	return repo.ResetHardTo(string(commit))
}
