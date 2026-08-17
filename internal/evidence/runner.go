package evidence

import (
	"io"
	"os"
	"os/exec"
	"runtime"
	"time"

	"github.com/hugues31/telos-sdd/internal/gitx"
)

// tailLimit bounds the captured suite output recorded in evidence.
const tailLimit = 2000

// tailWriter keeps the last tailLimit bytes of everything written.
type tailWriter struct {
	buf []byte
}

func (w *tailWriter) Write(p []byte) (int, error) {
	w.buf = append(w.buf, p...)
	if len(w.buf) > tailLimit {
		w.buf = w.buf[len(w.buf)-tailLimit:]
	}
	return len(p), nil
}

func (w *tailWriter) String() string { return string(w.buf) }

// SuiteRun is the outcome of running the configured commands on a tree.
type SuiteRun struct {
	Pass       bool
	ExitCode   int
	OutputTail string
	DurationMS int64
}

// RunSuiteOnTree materializes the exact tree in a throwaway detached
// worktree and runs the commands there. The candidate worktree is never
// mutated by a suite run — that structural property replaces V1's rollback
// bookkeeping. Output streams to echo (may be io.Discard) and is tail-caught
// for the record.
func RunSuiteOnTree(repo *gitx.Repo, tree gitx.OID, commands []string, echo io.Writer) (SuiteRun, error) {
	commit, err := repo.CommitTree(tree, nil, "telos: probe")
	if err != nil {
		return SuiteRun{}, err
	}
	dir, err := os.MkdirTemp("", "telos-probe-*")
	if err != nil {
		return SuiteRun{}, err
	}
	// worktree add wants to create the directory itself.
	os.RemoveAll(dir)
	if err := repo.WorktreeAddDetached(dir, commit); err != nil {
		return SuiteRun{}, err
	}
	defer func() {
		_ = repo.WorktreeRemove(dir)
		_ = os.RemoveAll(dir)
	}()

	tail := &tailWriter{}
	out := io.MultiWriter(echo, tail)
	start := time.Now()
	run := SuiteRun{Pass: true}
	for _, command := range commands {
		var cmd *exec.Cmd
		if runtime.GOOS == "windows" {
			cmd = exec.Command("cmd", "/C", command)
		} else {
			cmd = exec.Command("sh", "-c", command)
		}
		cmd.Dir = dir
		cmd.Stdout = out
		cmd.Stderr = out
		if err := cmd.Run(); err != nil {
			run.Pass = false
			if exitErr, ok := err.(*exec.ExitError); ok {
				run.ExitCode = exitErr.ExitCode()
			} else {
				run.ExitCode = -1
			}
			break
		}
	}
	run.DurationMS = time.Since(start).Milliseconds()
	run.OutputTail = tail.String()
	return run, nil
}
