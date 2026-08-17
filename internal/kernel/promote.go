package kernel

import (
	"encoding/json"
	"io"
	"runtime"
	"sort"
	"strings"
	"time"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/constraints"
	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/evidence"
	"github.com/hugues31/telos-sdd/internal/gitx"
	"github.com/hugues31/telos-sdd/internal/policy"
	"github.com/hugues31/telos-sdd/internal/provenance"
)

// readyState is everything the certification gates computed, reused by
// promotion so nothing is trusted across the two calls without recomputation
// (promote simply recomputes; KERNEL-007).
type readyState struct {
	doc         *ChangeDoc
	bundle      ReviewBundle
	target      contract.Contract
	foldedSpec  map[string][]byte
	entries     []EvidenceEntry
	verified    []string
	openIDs     []string
	suiteRecord *evidence.Record
	eff         policy.Effective
}

// ReadyReport is the result surface of `telos change ready`.
type ReadyReport struct {
	ID           string          `json:"id"`
	Digest       string          `json:"digest"`
	Kind         string          `json:"kind"`
	Evidence     []EvidenceEntry `json:"evidence"`
	Requirements []string        `json:"requirements_verified"`
	FindingsOpen []string        `json:"findings_open"`
}

// readyComputation runs every certification gate in order:
// state/tampering → exact base (KERNEL-001) → digest-bound approval
// (KERNEL-004) → open reds → blocking findings (KERNEL-006) → proof
// obligations (KERNEL-005) → suite evidence with content-addressed reuse
// (KERNEL-007).
func readyComputation(wt *gitx.Repo, cfg Config, echo io.Writer) (*readyState, error) {
	doc, err := LoadChange(wt)
	if err != nil {
		return nil, err
	}
	if doc.Status == ChangePromoted {
		return nil, coded.New("TELOS_CHANGE_STATE_INVALID", doc.ID+" is already promoted")
	}
	if err := requireEvidenceConfig(cfg); err != nil {
		return nil, err
	}
	bundle, err := reviewComputation(wt, doc)
	if err != nil {
		return nil, err
	}

	// KERNEL-001 — exact base.
	tip, err := wt.RevParse(doc.TargetBranch)
	if err != nil {
		return nil, err
	}
	if string(tip) != doc.Base {
		return nil, coded.New("TELOS_BASE_STALE", doc.TargetBranch+" moved since this Change's base; run `telos change rebase`, then retry")
	}

	// KERNEL-004 — digest-bound approval.
	approved := false
	for _, a := range doc.Approvals {
		if a.Kind == bundle.Kind && a.Digest == bundle.Digest {
			approved = true
			break
		}
	}
	if !approved {
		if len(doc.Approvals) > 0 {
			return nil, coded.New("TELOS_APPROVAL_STALE", "the recorded approval no longer matches the candidate content; review and approve again")
		}
		return nil, coded.New("TELOS_APPROVAL_REQUIRED", "no human approval recorded; run `telos change review` and present it")
	}

	// Open red witnesses forbid certification.
	if len(doc.RedWitnesses) > 0 {
		var reqs []string
		for req := range doc.RedWitnesses {
			reqs = append(reqs, req)
		}
		sort.Strings(reqs)
		return nil, coded.WithPaths("TELOS_RED_PENDING", "witnessed reds await their green; implement until the sealed tests pass", reqs)
	}

	// Policy loads before any policy-governed gate: a broken or
	// kernel-weakening policy forbids certification (KERNEL-008).
	eff, err := policy.Load(wt.WorkDir)
	if err != nil {
		return nil, err
	}

	// KERNEL-006 — blocking findings, human-set or policy-escalated.
	findings, err := LoadFindings(wt, doc.ID)
	if err != nil {
		return nil, err
	}
	if blocking := openBlocking(findings, eff); len(blocking) > 0 {
		return nil, coded.WithPaths("TELOS_FINDING_BLOCKING", "open blocking findings forbid certification; resolve them or fix the underlying issue", blocking)
	}

	target, folded, ops, err := targetContract(wt, doc)
	if err != nil {
		return nil, err
	}
	// Tier-1 structured constraints: a provably contradictory formalized
	// subset blocks certification.
	if err := constraints.Check(target); err != nil {
		return nil, err
	}
	tree, err := wt.TreeOf("HEAD")
	if err != nil {
		return nil, err
	}

	// KERNEL-005 — obligations. Every target requirement is cited by a test;
	// every requirement this change adds or modifies carries witnessed
	// red/green or gated adoption evidence.
	records, err := loadRecordsAt(wt, "HEAD")
	if err != nil {
		return nil, err
	}
	var unmet []string
	for _, id := range sortedRequirementIDs(target) {
		cited, err := citingTests(wt, cfg, tree, id)
		if err != nil {
			return nil, err
		}
		if len(cited) == 0 {
			unmet = append(unmet, id+" (no citing test)")
		}
	}
	changedReqs := map[string]bool{}
	for _, op := range ops {
		if (op.Kind == contract.OpAdd || op.Kind == contract.OpReplace) && strings.HasPrefix(op.ID, "REQ-") {
			changedReqs[op.ID] = true
		}
	}
	proven := map[string]bool{}
	for _, r := range records {
		if r.Change != doc.ID || r.Result.Status != "pass" {
			continue
		}
		if r.Kind == evidence.KindRedGreen || r.Adopted {
			for _, req := range r.Requirements {
				proven[req] = true
			}
		}
	}
	for req := range changedReqs {
		if !proven[req] {
			unmet = append(unmet, req+" (needs witnessed red/green or gated adoption)")
		}
	}
	if len(unmet) > 0 {
		sort.Strings(unmet)
		return nil, coded.WithPaths("TELOS_OBLIGATION_UNMET", "requirements lack their required evidence", unmet)
	}

	// Sealed bytes of this change's witnessed records must still be intact.
	var stale []string
	for _, r := range records {
		if r.Change != doc.ID || r.Kind != evidence.KindRedGreen || r.Witness == nil || r.Witness.Red == nil {
			continue
		}
		for _, s := range r.Witness.Red.SealedTests {
			if blob, err := wt.RevParse("HEAD:" + s.Path); err != nil || string(blob) != s.Blob {
				stale = append(stale, s.Path)
			}
		}
	}
	if len(stale) > 0 {
		sort.Strings(stale)
		return nil, coded.WithPaths("TELOS_RED_STALE", "sealed test bytes changed after their green witness; re-witness from red", stale)
	}

	// KERNEL-007 — the suite proof for this exact tree, reused when its
	// content-addressed closure is unchanged, recomputed otherwise.
	dep, err := evidence.TreeClosure(wt, tree)
	if err != nil {
		return nil, err
	}
	if specTree, terr := wt.TreeFromFiles(folded); terr == nil {
		if sub, serr := wt.SubtreeOf(string(specTree), "spec"); serr == nil {
			dep.Contract = string(sub)
		}
	}
	dep.Policy = eff.Hash
	prototype := evidence.Record{Kind: evidence.KindSuite, Command: strings.Join(cfg.TestCommands, " && "), Cwd: ".", DependsOn: dep}
	key := prototype.Key()

	var suiteRecord *evidence.Record
	reused := false
	for _, r := range records {
		if r.Kind == evidence.KindSuite && !r.Adopted && r.Reusable && r.Result.Status == "pass" && r.Key() == key {
			suiteRecord, reused = r, r.Change != doc.ID
			break
		}
	}
	if suiteRecord == nil {
		run, err := evidence.RunSuiteOnTree(wt, tree, cfg.TestCommands, echo)
		if err != nil {
			return nil, err
		}
		if !run.Pass {
			return nil, coded.New("TELOS_TESTS_FAILED", "the suite fails on the candidate tree ("+strings.TrimSpace(run.OutputTail)+")")
		}
		suiteRecord = &prototype
		suiteRecord.Schema = 1
		suiteRecord.Requirements = sortedRequirementIDs(target)
		suiteRecord.Result = evidence.Result{Status: "pass", ExitCode: run.ExitCode, OutputTail: run.OutputTail, DurationMS: run.DurationMS}
		suiteRecord.Reusable = true
		suiteRecord.Change = doc.ID
		suiteRecord.CreatedAt = time.Now().UTC().Format(time.RFC3339)
		suiteRecord.ID = "EVD-" + key[:12]
		if err := writeRecord(wt, doc, suiteRecord); err != nil {
			return nil, err
		}
		if _, err := wt.CommitAll("telos: suite evidence " + doc.ID); err != nil {
			return nil, err
		}
		if tree, err = wt.TreeOf("HEAD"); err != nil {
			return nil, err
		}
	}

	// Assemble the certificate's evidence entries: this change's records plus
	// the suite record (which may come from history).
	entryFor := func(r *evidence.Record) EvidenceEntry {
		blob := ""
		if oid, err := wt.RevParse("HEAD:" + changeDir(r.Change) + "/evidence/" + evidence.FileName(r.Key())); err == nil {
			blob = string(oid)
		}
		return EvidenceEntry{ID: r.ID, RecordBlob: blob, Reused: r.SurvivedRebase || r.Change != doc.ID, SourceChange: r.Change}
	}
	var entries []EvidenceEntry
	records, err = loadRecordsAt(wt, "HEAD")
	if err != nil {
		return nil, err
	}
	for _, r := range records {
		if r.Change == doc.ID && r.Kind != evidence.KindSuite {
			entries = append(entries, entryFor(r))
		}
	}
	suiteEntry := entryFor(suiteRecord)
	suiteEntry.Reused = reused
	entries = append(entries, suiteEntry)
	sort.Slice(entries, func(i, j int) bool { return entries[i].ID < entries[j].ID })

	doc.Status = ChangeReady
	if err := saveChange(wt, doc); err != nil {
		return nil, err
	}
	if _, err := wt.CommitAll("telos: ready " + doc.ID); err != nil {
		return nil, err
	}
	return &readyState{
		doc: doc, bundle: bundle, target: target, foldedSpec: folded,
		entries: entries, verified: sortedRequirementIDs(target),
		openIDs: openFindingIDs(findings), suiteRecord: suiteRecord, eff: eff,
	}, nil
}

// ReadyChange runs every certification gate without promoting.
func ReadyChange(wt *gitx.Repo, cfg Config, echo io.Writer) (ReadyReport, error) {
	state, err := readyComputation(wt, cfg, echo)
	if err != nil {
		return ReadyReport{}, err
	}
	return ReadyReport{
		ID: state.doc.ID, Digest: state.bundle.Digest, Kind: state.bundle.Kind,
		Evidence: state.entries, Requirements: state.verified, FindingsOpen: state.openIDs,
	}, nil
}

// buildProvenance derives the promotion's provenance document from the
// candidate's diff and its witnessed evidence. Best-effort: a nil return
// skips the file rather than failing the promotion.
func buildProvenance(wt *gitx.Repo, doc *ChangeDoc) *provenance.Doc {
	records, err := loadRecordsAt(wt, "HEAD")
	if err != nil {
		return nil
	}
	provenReqs := map[string]bool{}
	verifiedBy := map[string][]string{}
	evidenceIDs := map[string]string{}
	for _, r := range records {
		if r.Change != doc.ID || r.Result.Status != "pass" {
			continue
		}
		if r.Kind == evidence.KindRedGreen || r.Adopted {
			for _, req := range r.Requirements {
				provenReqs[req] = true
				evidenceIDs[req] = r.ID
				if r.Witness != nil && r.Witness.Red != nil {
					for _, s := range r.Witness.Red.SealedTests {
						verifiedBy[req] = append(verifiedBy[req], s.Path)
					}
				}
			}
		}
	}
	if len(provenReqs) == 0 {
		return nil
	}
	var reqs []string
	for req := range provenReqs {
		reqs = append(reqs, req)
	}

	changed, err := wt.DiffNames(doc.Base, "HEAD")
	if err != nil {
		return nil
	}
	code := map[string]provenance.FileVersions{}
	for _, p := range changed {
		if strings.HasPrefix(p, "changes/") || strings.HasPrefix(p, contract.Dir+"/") || p == ConfigFile {
			continue
		}
		head, err := wt.BlobAt("HEAD", p)
		if err != nil {
			continue // deleted file
		}
		base, _ := wt.BlobAt(doc.Base, p)
		code[p] = provenance.FileVersions{Base: base, Head: head}
	}
	built := provenance.Build(doc.ID, reqs, code, verifiedBy, evidenceIDs)
	return &built
}

// PromoteResult is the outcome of an atomic promotion.
type PromoteResult struct {
	ID      string `json:"id"`
	Commit  string `json:"commit"`
	Branch  string `json:"branch"`
	Root    string `json:"root,omitempty"`
	Cleaned bool   `json:"cleaned"`
}

// PromoteChange certifies and promotes the candidate: every gate is
// recomputed (KERNEL-007), the contract delta is folded into spec/, one
// commit is created from the exact base, and the certified branch and its
// certificate note move together in a single ref transaction (KERNEL-001
// under races: a lost CAS is TELOS_BASE_STALE, never partial state).
func PromoteChange(wt *gitx.Repo, cfg Config, version string, echo io.Writer) (PromoteResult, error) {
	var result PromoteResult
	state, err := readyComputation(wt, cfg, echo)
	if err != nil {
		return result, err
	}
	doc := state.doc
	result.ID = doc.ID
	result.Branch = doc.TargetBranch

	// Build the promotion tree: candidate HEAD with spec/ replaced by the
	// folded contract and the change record marked promoted.
	entries, err := wt.LsTreeEntries("HEAD")
	if err != nil {
		return result, err
	}
	var kept []gitx.TreeEntry
	for _, e := range entries {
		if e.Path == contract.Dir || strings.HasPrefix(e.Path, contract.Dir+"/") {
			continue
		}
		if e.Path == changeDir(doc.ID)+"/change.json" {
			continue
		}
		kept = append(kept, e)
	}
	for path, content := range state.foldedSpec {
		blob, err := wt.HashObject(content)
		if err != nil {
			return result, err
		}
		kept = append(kept, gitx.TreeEntry{Mode: "100644", Path: path, OID: blob})
	}
	// Record provenance from the verified transition: REQ → symbols/tests,
	// replacing V1's source annotations (docs/design-v2.md §11).
	if provDoc := buildProvenance(wt, doc); provDoc != nil {
		provBytes, err := json.MarshalIndent(provDoc, "", "  ")
		if err != nil {
			return result, err
		}
		provBlob, err := wt.HashObject(append(provBytes, '\n'))
		if err != nil {
			return result, err
		}
		kept = append(kept, gitx.TreeEntry{Mode: "100644", Path: changeDir(doc.ID) + "/provenance.json", OID: provBlob})
	}

	promotedDoc := *doc
	promotedDoc.Status = ChangePromoted
	promotedDoc.Review = nil
	docBytes, err := json.MarshalIndent(&promotedDoc, "", "  ")
	if err != nil {
		return result, err
	}
	docBlob, err := wt.HashObject(append(docBytes, '\n'))
	if err != nil {
		return result, err
	}
	kept = append(kept, gitx.TreeEntry{Mode: "100644", Path: changeDir(doc.ID) + "/change.json", OID: docBlob})
	promoTree, err := wt.TreeFromTreeEntries(kept)
	if err != nil {
		return result, err
	}
	title := doc.Title
	if title == "" {
		title = doc.Category
	}
	commit, err := wt.CommitTree(promoTree, []gitx.OID{gitx.OID(doc.Base)}, "telos: promote "+doc.ID+" — "+title)
	if err != nil {
		return result, err
	}

	// Seal the certificate.
	baseCert, err := LoadCertificate(wt, gitx.OID(doc.Base))
	if err != nil {
		return result, err
	}
	specTree, err := wt.SubtreeOf(string(promoTree), contract.Dir)
	if err != nil {
		return result, err
	}
	policyBlob, err := wt.RevParse(string(promoTree) + ":" + ConfigFile)
	if err != nil {
		return result, err
	}
	payload := CertPayload{
		Version:         1,
		Project:         baseCert.Payload.Project,
		Commit:          string(commit),
		Tree:            string(promoTree),
		ParentCertified: doc.Base,
		Change:          ChangeInfo{ID: doc.ID, Category: doc.Category, Base: doc.Base},
		Contract:        ContractInfo{Tree: string(specTree), Requirements: state.verified, DeltaFrom: baseCert.Payload.Contract.Tree},
		Policy:          PolicyInfo{Blob: string(policyBlob), Hash: state.eff.Hash},
		Approvals:       doc.Approvals,
		Verification:    Verification{Evidence: state.entries, RequirementsVerified: state.verified, FindingsOpen: state.openIDs},
		Toolchain:       Toolchain{Telos: version, Go: runtime.Version()},
		SealedAt:        time.Now().UTC().Format(time.RFC3339),
	}
	raw, err := marshalCanonical(payload)
	if err != nil {
		return result, err
	}
	env := certEnvelope{TelosCertificate: 1, Payload: raw, Seal: sealPayload(raw)}
	note, err := marshalCanonical(env)
	if err != nil {
		return result, err
	}
	newNotes, oldNotes, err := wt.NotesAddEntry(gitx.NotesRef, commit, note)
	if err != nil {
		return result, err
	}

	// Remember whether the certified root worktree can be fast-forwarded:
	// it must be clean against the OLD tip, checked before the refs move.
	rootPath := ""
	var rootRepo *gitx.Repo
	rootClean := false
	if worktrees, err := wt.WorktreeList(); err == nil {
		for _, info := range worktrees {
			if info.Branch == doc.TargetBranch {
				rootPath = info.Path
				if r, err := gitx.Open(info.Path); err == nil {
					rootRepo = r
					if dirty, derr := r.DirtyPaths(); derr == nil && dirty == nil {
						rootClean = true
					}
				}
			}
		}
	}

	// The atomic step: branch and certificate move together, CAS on the base.
	updates := []gitx.RefUpdate{
		{Ref: "refs/heads/" + doc.TargetBranch, New: commit, Old: gitx.OID(doc.Base)},
	}
	if oldNotes == "" {
		updates = append(updates, gitx.RefUpdate{Ref: gitx.NotesRef, New: newNotes, Create: true})
	} else {
		updates = append(updates, gitx.RefUpdate{Ref: gitx.NotesRef, New: newNotes, Old: oldNotes})
	}
	if err := wt.RefTransaction(updates); err != nil {
		return result, coded.New("TELOS_BASE_STALE", doc.TargetBranch+" or its certificates moved during promotion; rebase and retry ("+err.Error()+")")
	}
	result.Commit = string(commit)

	// Fast-forward the certified root worktree when it was clean; a dirty
	// root stays untouched and shows up as corrupted-with-salvage instead.
	if rootRepo != nil && rootClean {
		if err := rootRepo.ResetHardTo(string(commit)); err == nil {
			result.Root = rootPath
		}
	}

	// Cleanup: remove the candidate. Failure is benign (doctor repairs).
	if rootRepo != nil {
		if err := rootRepo.WorktreeRemove(wt.WorkDir); err == nil {
			_ = rootRepo.BranchDelete(doc.Branch)
			result.Cleaned = true
		}
	}
	return result, nil
}
