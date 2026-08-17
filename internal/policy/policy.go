// Package policy loads and evaluates certification policy: an embedded,
// closed kernel schema and floor (non-weakenable by construction —
// KERNEL-008 is structural, not reviewed-for) unified with the project's
// policies/*.cue. The unified value's canonical export is hashed into the
// certificate, so evidence bound to a policy hash stales automatically when
// the rules that governed it change.
package policy

import (
	"crypto/sha256"
	"embed"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"cuelang.org/go/cue"
	"cuelang.org/go/cue/cuecontext"

	"github.com/hugues31/telos-sdd/internal/coded"
)

//go:embed kernel/*.cue
var kernelFS embed.FS

const kernelSchemaVersion = "1"

// EvidenceClassRule is the required evidence set for one requirement class.
type EvidenceClassRule struct {
	RedGreen    bool `json:"red_green"`
	Adversarial bool `json:"adversarial"`
	Benchmark   bool `json:"benchmark"`
	Mutation    bool `json:"mutation"`
}

// EscalationRule deterministically escalates critic findings.
type EscalationRule struct {
	MinConfidence    float64 `json:"min_confidence"`
	ProposedSeverity string  `json:"proposed_severity"`
	Action           string  `json:"action"` // annotate|require_human|block
}

// Effective is the decoded, unified policy.
type Effective struct {
	Hash       string                       `json:"hash"`
	Evidence   map[string]EvidenceClassRule `json:"evidence"`
	Escalation []EscalationRule             `json:"escalation"`
	Protected  []string                     `json:"protected"`
}

// kernelValue compiles the embedded kernel files as ONE source so the
// definitions of schema.cue resolve inside floor.cue.
func kernelValue(ctx *cue.Context) (cue.Value, error) {
	entries, err := kernelFS.ReadDir("kernel")
	if err != nil {
		return cue.Value{}, err
	}
	names := make([]string, 0, len(entries))
	for _, e := range entries {
		names = append(names, e.Name())
	}
	sort.Strings(names)
	var combined strings.Builder
	for _, name := range names {
		data, err := kernelFS.ReadFile("kernel/" + name)
		if err != nil {
			return cue.Value{}, err
		}
		combined.Write(stripPackageLine(data))
		combined.WriteString("\n")
	}
	v := ctx.CompileString(combined.String(), cue.Filename("telos-kernel.cue"))
	if v.Err() != nil {
		return cue.Value{}, v.Err()
	}
	return v, nil
}

func stripPackageLine(data []byte) []byte {
	lines := strings.Split(string(data), "\n")
	for i, line := range lines {
		if strings.HasPrefix(strings.TrimSpace(line), "package ") {
			lines[i] = ""
			break
		}
	}
	return []byte(strings.Join(lines, "\n"))
}

// Load compiles the kernel policy unified with the project's policies/*.cue.
// A project value conflicting with a kernel floor is
// TELOS_POLICY_WEAKENS_KERNEL; anything that does not compile is
// TELOS_POLICY_INVALID.
func Load(root string) (Effective, error) {
	ctx := cuecontext.New()
	value, err := kernelValue(ctx)
	if err != nil {
		return Effective{}, coded.New("TELOS_POLICY_INVALID", "embedded kernel policy does not compile: "+err.Error())
	}

	projectDir := filepath.Join(root, "policies")
	entries, err := os.ReadDir(projectDir)
	if err == nil {
		names := make([]string, 0, len(entries))
		for _, e := range entries {
			if !e.IsDir() && strings.HasSuffix(e.Name(), ".cue") {
				names = append(names, e.Name())
			}
		}
		sort.Strings(names)
		for _, name := range names {
			data, err := os.ReadFile(filepath.Join(projectDir, name))
			if err != nil {
				return Effective{}, err
			}
			v := ctx.CompileString(string(stripPackageLine(data)), cue.Filename("policies/"+name), cue.Scope(value))
			if v.Err() != nil {
				return Effective{}, coded.New("TELOS_POLICY_INVALID", "policies/"+name+" does not compile: "+v.Err().Error())
			}
			value = value.Unify(v)
		}
	}
	if err := value.Validate(cue.Concrete(false)); err != nil {
		return Effective{}, coded.New("TELOS_POLICY_WEAKENS_KERNEL", "project policy conflicts with a kernel floor; kernel invariants cannot be weakened: "+err.Error())
	}

	var decoded struct {
		Evidence   map[string]EvidenceClassRule `json:"evidence"`
		Escalation struct {
			Kernel  []EscalationRule `json:"kernel"`
			Project []EscalationRule `json:"project"`
		} `json:"escalation"`
		Protected map[string]bool `json:"protected"`
	}
	exported, err := value.MarshalJSON()
	if err != nil {
		return Effective{}, coded.New("TELOS_POLICY_WEAKENS_KERNEL", "project policy conflicts with a kernel floor: "+err.Error())
	}
	if err := json.Unmarshal(exported, &decoded); err != nil {
		return Effective{}, coded.New("TELOS_POLICY_INVALID", "policy does not decode: "+err.Error())
	}

	eff := Effective{Evidence: decoded.Evidence}
	eff.Escalation = append(decoded.Escalation.Kernel, decoded.Escalation.Project...)
	for path, on := range decoded.Protected {
		if on {
			eff.Protected = append(eff.Protected, path)
		}
	}
	sort.Strings(eff.Protected)

	// The hash covers the canonical export plus the kernel schema version:
	// identical rules hash identically regardless of file layout.
	canonical, err := canonicalJSON(exported)
	if err != nil {
		return Effective{}, err
	}
	sum := sha256.Sum256(append(canonical, []byte("\nkernel:"+kernelSchemaVersion)...))
	eff.Hash = hex.EncodeToString(sum[:])
	return eff, nil
}

// Escalate reports whether a finding must block certification under the
// deterministic rules (strictest-wins: any matching rule with action "block"
// blocks).
func (e Effective) Escalate(proposedSeverity string, confidence float64) string {
	action := ""
	rank := map[string]int{"": 0, "annotate": 1, "require_human": 2, "block": 3}
	for _, rule := range e.Escalation {
		if rule.ProposedSeverity == proposedSeverity && confidence >= rule.MinConfidence {
			if rank[rule.Action] > rank[action] {
				action = rule.Action
			}
		}
	}
	return action
}

// canonicalJSON re-marshals with sorted keys for a stable hash.
func canonicalJSON(data []byte) ([]byte, error) {
	var v any
	if err := json.Unmarshal(data, &v); err != nil {
		return nil, err
	}
	return json.Marshal(v) // encoding/json sorts map keys
}
