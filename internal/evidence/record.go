// Package evidence implements the content-addressed evidence model: records
// whose reuse key is the hash of their exact inputs (KERNEL-007 means
// "recompute the validity of the exact candidate", not "rerun every proof
// blindly"). Suite runs happen in throwaway detached worktrees of the exact
// tree under proof — the candidate is never mutated by a run.
package evidence

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
)

// Evidence kinds. benchmark is never reusable; witnessed_red_green carries
// the sealed red/green witness.
const (
	KindSuite     = "suite"
	KindRedGreen  = "witnessed_red_green"
	KindBenchmark = "benchmark"
)

// Toolchain pins the environment a record was produced under.
type Toolchain struct {
	Go   string `json:"go"`
	OS   string `json:"os"`
	Arch string `json:"arch"`
}

// DependsOn is the content-addressed dependency closure of a record: if no
// input in the closure changed, the record may be reused; when dependencies
// cannot be determined, the closure falls back to the whole tracked tree
// (conservative invalidation).
type DependsOn struct {
	Closure       string    `json:"closure"` // go_packages|tracked_tree
	ClosureDigest string    `json:"closure_digest"`
	Packages      []string  `json:"packages,omitempty"`
	Contract      string    `json:"contract,omitempty"`
	Policy        string    `json:"policy,omitempty"`
	Toolchain     Toolchain `json:"toolchain"`
}

// Result is the observed outcome of the record's command.
type Result struct {
	Status     string `json:"status"` // pass|fail
	ExitCode   int    `json:"exit_code"`
	OutputTail string `json:"output_tail"`
	DurationMS int64  `json:"duration_ms"`
}

// SealedTest is one test file sealed as red evidence: its exact blob OID.
type SealedTest struct {
	Path string `json:"path"`
	Blob string `json:"blob"`
}

// RedWitness records the broker witnessing the sealed tests failing on a
// tree whose baseline (the same tree without them) was green.
type RedWitness struct {
	BaselineTree string       `json:"baseline_tree"`
	FailedTree   string       `json:"failed_tree"`
	SealedTests  []SealedTest `json:"sealed_tests"`
	OutputTail   string       `json:"output_tail"`
}

// GreenWitness records the same sealed bytes passing.
type GreenWitness struct {
	Tree              string `json:"tree"`
	SealedTestsIntact bool   `json:"sealed_tests_intact"`
}

// Witness pairs the red and green halves of a witnessed cycle.
type Witness struct {
	Red   *RedWitness   `json:"red,omitempty"`
	Green *GreenWitness `json:"green,omitempty"`
}

// Record is one committed evidence record
// (changes/CHG-NNN/evidence/EVD-*.json).
type Record struct {
	Schema       int       `json:"evidence"`
	ID           string    `json:"id"`
	Kind         string    `json:"kind"`
	Requirements []string  `json:"requirements"`
	Command      string    `json:"command"`
	Cwd          string    `json:"cwd"`
	DependsOn    DependsOn `json:"depends_on"`
	Result       Result    `json:"result"`
	Witness      *Witness  `json:"witness,omitempty"`
	Reusable     bool      `json:"reusable"`
	Adopted      bool      `json:"adopted,omitempty"`
	Change       string    `json:"change"`
	CreatedAt    string    `json:"created_at"`
}

// Key is the content address of the record's inputs: two records with the
// same key proved the same thing about the same bytes.
func (r *Record) Key() string {
	payload := struct {
		Kind      string    `json:"kind"`
		Command   string    `json:"command"`
		Cwd       string    `json:"cwd"`
		DependsOn DependsOn `json:"depends_on"`
	}{r.Kind, r.Command, r.Cwd, r.DependsOn}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	_ = enc.Encode(payload)
	sum := sha256.Sum256(bytes.TrimRight(buf.Bytes(), "\n"))
	return hex.EncodeToString(sum[:])
}

// FileName returns the committed record file name for a key.
func FileName(key string) string {
	return "EVD-" + key[:12] + ".json"
}
