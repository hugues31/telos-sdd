package kernel

import (
	"encoding/json"
	"errors"
	"sort"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/gitx"
)

// Transition categories.
const (
	CategoryBehaviorChange     = "behavior_change"
	CategoryBehaviorPreserving = "behavior_preserving"
	CategoryPolicyChange       = "policy_change"
	CategoryGenesis            = "genesis"
)

// ProjectInfo identifies the project inside a certificate.
type ProjectInfo struct {
	ID      string `json:"id"`
	Genesis string `json:"genesis"`
}

// ChangeInfo names the transition that produced a certificate.
type ChangeInfo struct {
	ID       string `json:"id"`
	Category string `json:"category"`
	Base     string `json:"base"`
}

// ContractInfo binds the certified contract.
type ContractInfo struct {
	Tree         string   `json:"tree"`
	Requirements []string `json:"requirements"`
	DeltaFrom    string   `json:"delta_from"`
}

// PolicyInfo binds the certified policy content.
type PolicyInfo struct {
	Blob string `json:"blob"`
	Hash string `json:"hash"`
}

// Approval records one digest-bound human decision.
type Approval struct {
	Kind   string `json:"kind"` // contract|preserving_claim|adoption|policy|reset
	Digest string `json:"digest"`
	At     string `json:"at"`
}

// EvidenceEntry references one evidence record backing the certificate.
type EvidenceEntry struct {
	ID           string `json:"id"`
	RecordBlob   string `json:"record_blob"`
	Reused       bool   `json:"reused"`
	SourceChange string `json:"source_change"`
}

// Verification summarizes what was recomputed or reused at certification.
type Verification struct {
	Evidence             []EvidenceEntry `json:"evidence"`
	RequirementsVerified []string        `json:"requirements_verified"`
	FindingsOpen         []string        `json:"findings_open"`
}

// Toolchain records the certifying toolchain.
type Toolchain struct {
	Telos string `json:"telos"`
	Go    string `json:"go"`
}

// CertPayload is the sealed content of a certificate. Its canonical bytes are
// produced by marshalCanonical exactly once, at sealing time; verification
// re-extracts the stored bytes instead of re-canonicalizing.
type CertPayload struct {
	Version         int          `json:"version"`
	Project         ProjectInfo  `json:"project"`
	Commit          string       `json:"commit"`
	Tree            string       `json:"tree"`
	ParentCertified string       `json:"parent_certified"`
	Change          ChangeInfo   `json:"change"`
	Contract        ContractInfo `json:"contract"`
	Policy          PolicyInfo   `json:"policy"`
	Approvals       []Approval   `json:"approvals"`
	Verification    Verification `json:"verification"`
	Toolchain       Toolchain    `json:"toolchain"`
	SealedAt        string       `json:"sealed_at"`
}

type certEnvelope struct {
	TelosCertificate int             `json:"telos_certificate"`
	Payload          json.RawMessage `json:"payload"`
	Seal             SealInfo        `json:"seal"`
}

// Certificate is a loaded certificate note. payloadRaw holds the exact sealed
// bytes as stored, so MAC verification never depends on re-canonicalization.
type Certificate struct {
	Payload    CertPayload
	Seal       SealInfo
	payloadRaw []byte
}

func certInvalid(reason string) error {
	return coded.New("TELOS_CERTIFICATE_INVALID", "certificate invalid: "+reason)
}

// LoadCertificate reads the certificate note attached to commit.
func LoadCertificate(repo *gitx.Repo, commit gitx.OID) (*Certificate, error) {
	raw, err := repo.NoteShow(gitx.NotesRef, commit)
	if err != nil {
		if errors.Is(err, gitx.ErrNoNote) {
			return nil, certInvalid("commit " + short(commit) + " carries no certificate note")
		}
		return nil, err
	}
	var env certEnvelope
	if err := json.Unmarshal(raw, &env); err != nil {
		return nil, certInvalid("note is not a certificate envelope: " + err.Error())
	}
	if env.TelosCertificate != 1 {
		return nil, certInvalid("unsupported envelope version")
	}
	cert := &Certificate{Seal: env.Seal, payloadRaw: []byte(env.Payload)}
	if err := json.Unmarshal(env.Payload, &cert.Payload); err != nil {
		return nil, certInvalid("payload does not parse: " + err.Error())
	}
	return cert, nil
}

// Validate checks the seal and the binding of the certificate to the commit
// it is attached to: the MAC covers the exact stored payload bytes, the
// payload names this very commit (a note cannot be copied onto another one),
// and the recorded tree is the commit's tree.
func (c *Certificate) Validate(commit, tree gitx.OID) error {
	if !sealValid(c.payloadRaw, c.Seal) {
		return certInvalid("seal does not verify")
	}
	if c.Payload.Version != 1 {
		return certInvalid("unsupported payload version")
	}
	if c.Payload.Commit != string(commit) {
		return certInvalid("certificate is bound to commit " + short(gitx.OID(c.Payload.Commit)) + ", not " + short(commit))
	}
	if c.Payload.Tree != string(tree) {
		return certInvalid("certificate tree does not match the commit tree")
	}
	return nil
}

// writeCertificate seals a payload and attaches it as the commit's note.
// It is deliberately unexported: the only writers are the kernel's own
// transitions (genesis now, Seal(VerifiedTransition) from M3 on) — there is
// no path that certifies an arbitrary current state (KERNEL-002).
func writeCertificate(repo *gitx.Repo, payload CertPayload) (*Certificate, error) {
	raw, err := marshalCanonical(payload)
	if err != nil {
		return nil, err
	}
	env := certEnvelope{TelosCertificate: 1, Payload: json.RawMessage(raw), Seal: sealPayload(raw)}
	note, err := marshalCanonical(env)
	if err != nil {
		return nil, err
	}
	if err := repo.NoteAdd(gitx.NotesRef, gitx.OID(payload.Commit), note); err != nil {
		return nil, err
	}
	return &Certificate{Payload: payload, Seal: env.Seal, payloadRaw: raw}, nil
}

// contractFilesAt collects spec/** contents from a revision's tree.
func contractFilesAt(repo *gitx.Repo, rev string) (map[string][]byte, error) {
	files, err := repo.LsTree(rev)
	if err != nil {
		return nil, err
	}
	out := map[string][]byte{}
	for path, oid := range files {
		if path == contract.Dir || len(path) > len(contract.Dir) && path[:len(contract.Dir)+1] == contract.Dir+"/" {
			content, err := repo.CatBlob(oid)
			if err != nil {
				return nil, err
			}
			out[path] = content
		}
	}
	return out, nil
}

func sortedRequirementIDs(c contract.Contract) []string {
	out := make([]string, 0, len(c.Requirements))
	for id := range c.Requirements {
		out = append(out, id)
	}
	sort.Strings(out)
	return out
}

func short(oid gitx.OID) string {
	if len(oid) > 12 {
		return string(oid[:12])
	}
	return string(oid)
}
