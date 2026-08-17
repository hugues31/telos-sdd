package kernel

import (
	"errors"
	"io"
	"os/exec"
	"runtime"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/gitx"
)

// VerifyReport is the result of a successful verification.
type VerifyReport struct {
	Commit       string `json:"commit"`
	Change       string `json:"change"`
	Intents      int    `json:"intents"`
	Requirements int    `json:"requirements"`
	Decisions    int    `json:"decisions"`
}

// Verify recomputes the validity of the certified root: the certificate
// seals this exact commit and tree, the worktree matches it byte for byte,
// the contract is structurally valid, and the configured suite is green.
// (Per-requirement proof obligations arrive with the evidence model at M3.)
func Verify(repo *gitx.Repo, cfg Config, stdout, stderr io.Writer) (VerifyReport, error) {
	var report VerifyReport

	head, err := repo.Head()
	if errors.Is(err, gitx.ErrNoCommits) {
		return report, coded.New("TELOS_CERTIFICATE_INVALID", "no certified state: the repository has no commits; run `telos init`")
	} else if err != nil {
		return report, err
	}
	cert, err := LoadCertificate(repo, head)
	if err != nil {
		return report, err
	}
	tree, err := repo.TreeOf("HEAD")
	if err != nil {
		return report, err
	}
	if err := cert.Validate(head, tree); err != nil {
		return report, err
	}

	dirty, err := repo.DirtyPaths()
	if err != nil {
		return report, err
	}
	if len(dirty) > 0 {
		return report, coded.WithPaths("TELOS_STATE_CORRUPTED", "worktree diverged from the certified state; capture the diff as a Change (salvage) or restore", dirty)
	}

	files, err := contractFilesAt(repo, "HEAD")
	if err != nil {
		return report, err
	}
	parsed, problems := contract.Parse(files)
	if len(problems) > 0 {
		return report, coded.WithPaths("TELOS_CONTRACT_INVALID", "certified contract is structurally invalid", problems)
	}

	if err := RunTestCommands(repo.WorkDir, cfg.TestCommands, stdout, stderr); err != nil {
		return report, coded.New("TELOS_TESTS_FAILED", "test commands failed: "+err.Error())
	}
	// A suite that mutates tracked files silently invalidates what was just
	// verified; that is corruption, reported with the offending paths.
	dirty, err = repo.DirtyPaths()
	if err != nil {
		return report, err
	}
	if len(dirty) > 0 {
		return report, coded.WithPaths("TELOS_STATE_CORRUPTED", "the test commands mutated tracked files", dirty)
	}

	report = VerifyReport{
		Commit:       string(head),
		Change:       cert.Payload.Change.ID,
		Intents:      len(parsed.Intents),
		Requirements: len(parsed.Requirements),
		Decisions:    len(parsed.Decisions),
	}
	return report, nil
}

// RunTestCommands runs each configured command through the platform shell in
// root, streaming output; the first failure aborts.
func RunTestCommands(root string, commands []string, stdout, stderr io.Writer) error {
	for _, command := range commands {
		var cmd *exec.Cmd
		if runtime.GOOS == "windows" {
			cmd = exec.Command("cmd", "/C", command)
		} else {
			cmd = exec.Command("sh", "-c", command)
		}
		cmd.Dir = root
		cmd.Stdout = stdout
		cmd.Stderr = stderr
		if err := cmd.Run(); err != nil {
			return errors.New(command + ": " + err.Error())
		}
	}
	return nil
}
