package kernel

import (
	"encoding/json"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/contract"
	"github.com/hugues31/telos-sdd/internal/evidence"
	"github.com/hugues31/telos-sdd/internal/gitx"
	"github.com/hugues31/telos-sdd/internal/glob"
	"github.com/hugues31/telos-sdd/internal/policy"
)

// targetContract returns the contract this Change certifies against: the
// base contract with the delta folded in (behavior_change), or the base
// contract itself (behavior_preserving).
func targetContract(wt *gitx.Repo, doc *ChangeDoc) (contract.Contract, map[string][]byte, []contract.Op, error) {
	baseSpec, err := contractFilesAt(wt, doc.Base)
	if err != nil {
		return contract.Contract{}, nil, nil, err
	}
	deltaBytes, err := os.ReadFile(filepath.Join(wt.WorkDir, filepath.FromSlash(changeDir(doc.ID)), "contract.delta.md"))
	if err != nil {
		return contract.Contract{}, nil, nil, coded.New("TELOS_CHANGE_UNKNOWN", doc.ID+" has no contract.delta.md")
	}
	ops, err := contract.ParseDelta(deltaBytes)
	if err != nil {
		return contract.Contract{}, nil, nil, coded.New("TELOS_CONTRACT_INVALID", err.Error())
	}
	files := baseSpec
	if doc.Category == CategoryBehaviorChange && len(ops) > 0 {
		if files, err = contract.Fold(baseSpec, ops); err != nil {
			return contract.Contract{}, nil, nil, coded.New("TELOS_CONTRACT_INVALID", err.Error())
		}
	}
	parsed, problems := contract.Parse(files)
	if len(problems) > 0 {
		return contract.Contract{}, nil, nil, coded.WithPaths("TELOS_CONTRACT_INVALID", "the target contract is structurally invalid", problems)
	}
	return parsed, files, ops, nil
}

// citingTests lists the configured test files of a tree whose content cites
// the requirement.
func citingTests(wt *gitx.Repo, cfg Config, tree gitx.OID, reqID string) ([]evidence.SealedTest, error) {
	files, err := wt.LsTree(string(tree))
	if err != nil {
		return nil, err
	}
	var sealed []evidence.SealedTest
	for path, oid := range files {
		if !glob.MatchAny(cfg.TestFiles, path) {
			continue
		}
		content, err := wt.CatBlob(oid)
		if err != nil {
			return nil, err
		}
		for _, ref := range contract.ReqRefs(content) {
			if ref == reqID {
				sealed = append(sealed, evidence.SealedTest{Path: path, Blob: string(oid)})
				break
			}
		}
	}
	sort.Slice(sealed, func(i, j int) bool { return sealed[i].Path < sealed[j].Path })
	return sealed, nil
}

func requireEvidenceConfig(cfg Config) error {
	if len(cfg.TestCommands) == 0 || len(cfg.TestFiles) == 0 {
		return coded.New("TELOS_CONFIG_INVALID", "test_commands and test_files must be configured in telos.toml before requirements can be proven")
	}
	return nil
}

// EvidenceRed witnesses a requirement's citing tests failing: the same tree
// WITHOUT those tests must be green (the baseline), the tree WITH them must
// be red. Sealing is by blob OID.
func EvidenceRed(wt *gitx.Repo, cfg Config, reqID string, echo io.Writer) (*ChangeDoc, *evidence.RedWitness, error) {
	doc, err := LoadChange(wt)
	if err != nil {
		return nil, nil, err
	}
	if doc.Status == ChangePromoted {
		return nil, nil, coded.New("TELOS_CHANGE_STATE_INVALID", doc.ID+" is already promoted")
	}
	if err := requireEvidenceConfig(cfg); err != nil {
		return nil, nil, err
	}
	target, _, _, err := targetContract(wt, doc)
	if err != nil {
		return nil, nil, err
	}
	if target.Requirements[reqID] == nil {
		return nil, nil, coded.New("TELOS_REQUIREMENT_UNKNOWN", reqID+" does not exist in the target contract; declare it in contract.delta.md first")
	}

	if _, err := wt.CommitAll("telos: snapshot " + doc.ID); err != nil {
		return nil, nil, err
	}
	tree, err := wt.TreeOf("HEAD")
	if err != nil {
		return nil, nil, err
	}
	sealed, err := citingTests(wt, cfg, tree, reqID)
	if err != nil {
		return nil, nil, err
	}
	if len(sealed) == 0 {
		return nil, nil, coded.New("TELOS_TEST_FIRST", "no configured test file cites "+reqID+"; submit the failing test before any implementation")
	}

	// Baseline: this exact tree with the citing tests reverted to their base
	// content (or absent). A new test is only evidence on a green baseline.
	entries, err := wt.LsTree(string(tree))
	if err != nil {
		return nil, nil, err
	}
	for _, s := range sealed {
		if baseBlob, err := wt.RevParse(doc.Base + ":" + s.Path); err == nil {
			entries[s.Path] = baseBlob
		} else {
			delete(entries, s.Path)
		}
	}
	baselineTree, err := wt.TreeFromEntries(entries)
	if err != nil {
		return nil, nil, err
	}
	baseline, err := evidence.RunSuiteOnTree(wt, baselineTree, cfg.TestCommands, echo)
	if err != nil {
		return nil, nil, err
	}
	if !baseline.Pass {
		return nil, nil, coded.New("TELOS_BASELINE_RED", "a new test is only evidence on a green baseline; make the suite pass without the citing tests first ("+strings.TrimSpace(baseline.OutputTail)+")")
	}

	run, err := evidence.RunSuiteOnTree(wt, tree, cfg.TestCommands, echo)
	if err != nil {
		return nil, nil, err
	}
	if run.Pass {
		return nil, nil, coded.New("TELOS_RED_EXPECTED", "the citing tests already pass, so they prove nothing; strengthen them, or use `telos evidence adopt` for behavior the code already has (human-gated)")
	}

	witness := evidence.RedWitness{
		BaselineTree: string(baselineTree),
		FailedTree:   string(tree),
		SealedTests:  sealed,
		OutputTail:   run.OutputTail,
	}
	if doc.RedWitnesses == nil {
		doc.RedWitnesses = map[string]evidence.RedWitness{}
	}
	doc.RedWitnesses[reqID] = witness
	if err := saveChange(wt, doc); err != nil {
		return nil, nil, err
	}
	if _, err := wt.CommitAll("telos: witness red " + reqID); err != nil {
		return nil, nil, err
	}
	return doc, &witness, nil
}

// EvidenceGreen verifies the sealed bytes are intact and the suite now
// passes, then commits the witnessed_red_green record.
func EvidenceGreen(wt *gitx.Repo, cfg Config, reqID string, echo io.Writer) (*ChangeDoc, *evidence.Record, error) {
	doc, err := LoadChange(wt)
	if err != nil {
		return nil, nil, err
	}
	if err := requireEvidenceConfig(cfg); err != nil {
		return nil, nil, err
	}
	witness, ok := doc.RedWitnesses[reqID]
	if !ok {
		return nil, nil, coded.New("TELOS_TEST_FIRST", "no witnessed red for "+reqID+"; run `telos evidence red --req "+reqID+"` first")
	}

	if _, err := wt.CommitAll("telos: snapshot " + doc.ID); err != nil {
		return nil, nil, err
	}
	tree, err := wt.TreeOf("HEAD")
	if err != nil {
		return nil, nil, err
	}
	var stale []string
	for _, s := range witness.SealedTests {
		if blob, err := wt.RevParse("HEAD:" + s.Path); err != nil || string(blob) != s.Blob {
			stale = append(stale, s.Path)
		}
	}
	if len(stale) > 0 {
		return nil, nil, coded.WithPaths("TELOS_RED_STALE", "sealed test bytes changed since the red witness; re-witness from red — the seal is never edited to fit", stale)
	}

	run, err := evidence.RunSuiteOnTree(wt, tree, cfg.TestCommands, echo)
	if err != nil {
		return nil, nil, err
	}
	if !run.Pass {
		return nil, nil, coded.New("TELOS_RED_PENDING", "the sealed tests still fail; only the implementation may turn red into green ("+strings.TrimSpace(run.OutputTail)+")")
	}

	record, err := buildRecord(wt, cfg, doc, evidence.KindRedGreen, []string{reqID}, sealedPaths(witness.SealedTests), tree, run)
	if err != nil {
		return nil, nil, err
	}
	record.Witness = &evidence.Witness{Red: &witness, Green: &evidence.GreenWitness{Tree: string(tree), SealedTestsIntact: true}}
	if err := writeRecord(wt, doc, record); err != nil {
		return nil, nil, err
	}
	delete(doc.RedWitnesses, reqID)
	if err := saveChange(wt, doc); err != nil {
		return nil, nil, err
	}
	if _, err := wt.CommitAll("telos: witness green " + reqID); err != nil {
		return nil, nil, err
	}
	return doc, record, nil
}

// EvidenceAdopt records already-correct behavior as proof: the citing test
// must pass immediately. It is human-gated at the guard, because such a test
// can never be witnessed failing.
func EvidenceAdopt(wt *gitx.Repo, cfg Config, reqID string, echo io.Writer) (*ChangeDoc, *evidence.Record, error) {
	doc, err := LoadChange(wt)
	if err != nil {
		return nil, nil, err
	}
	if err := requireEvidenceConfig(cfg); err != nil {
		return nil, nil, err
	}
	target, _, _, err := targetContract(wt, doc)
	if err != nil {
		return nil, nil, err
	}
	if target.Requirements[reqID] == nil {
		return nil, nil, coded.New("TELOS_REQUIREMENT_UNKNOWN", reqID+" does not exist in the target contract")
	}
	if _, err := wt.CommitAll("telos: snapshot " + doc.ID); err != nil {
		return nil, nil, err
	}
	tree, err := wt.TreeOf("HEAD")
	if err != nil {
		return nil, nil, err
	}
	sealed, err := citingTests(wt, cfg, tree, reqID)
	if err != nil {
		return nil, nil, err
	}
	if len(sealed) == 0 {
		return nil, nil, coded.New("TELOS_TEST_FIRST", "no configured test file cites "+reqID+"; adoption still needs a documentation test")
	}
	run, err := evidence.RunSuiteOnTree(wt, tree, cfg.TestCommands, echo)
	if err != nil {
		return nil, nil, err
	}
	if !run.Pass {
		return nil, nil, coded.New("TELOS_TESTS_FAILED", "adoption requires the suite to pass with the documentation test in place ("+strings.TrimSpace(run.OutputTail)+")")
	}
	record, err := buildRecord(wt, cfg, doc, evidence.KindSuite, []string{reqID}, sealedPaths(sealed), tree, run)
	if err != nil {
		return nil, nil, err
	}
	record.Adopted = true
	if err := writeRecord(wt, doc, record); err != nil {
		return nil, nil, err
	}
	if _, err := wt.CommitAll("telos: adopt " + reqID); err != nil {
		return nil, nil, err
	}
	return doc, record, nil
}

func sealedPaths(sealed []evidence.SealedTest) []string {
	out := make([]string, len(sealed))
	for i, s := range sealed {
		out[i] = s.Path
	}
	return out
}

// buildRecord assembles a record with its closure, contract, and policy
// bindings for the given tree.
func buildRecord(wt *gitx.Repo, cfg Config, doc *ChangeDoc, kind string, reqs, closureFiles []string, tree gitx.OID, run evidence.SuiteRun) (*evidence.Record, error) {
	dep, err := evidence.ClosureFor(wt, tree, wt.WorkDir, cfg.EffectiveClosure(wt.WorkDir), closureFiles)
	if err != nil {
		return nil, err
	}
	if _, files, _, err := targetContract(wt, doc); err == nil {
		if specTree, terr := wt.TreeFromFiles(files); terr == nil {
			if sub, serr := wt.SubtreeOf(string(specTree), "spec"); serr == nil {
				dep.Contract = string(sub)
			}
		}
	}
	if eff, err := policy.Load(wt.WorkDir); err == nil {
		dep.Policy = eff.Hash
	} else if policyBlob, berr := wt.RevParse("HEAD:" + ConfigFile); berr == nil {
		dep.Policy = string(policyBlob)
	}
	status := "fail"
	if run.Pass {
		status = "pass"
	}
	record := &evidence.Record{
		Schema:       1,
		Kind:         kind,
		Requirements: reqs,
		Command:      strings.Join(cfg.TestCommands, " && "),
		Cwd:          ".",
		DependsOn:    dep,
		Result:       evidence.Result{Status: status, ExitCode: run.ExitCode, OutputTail: run.OutputTail, DurationMS: run.DurationMS},
		Reusable:     kind != evidence.KindBenchmark,
		Change:       doc.ID,
		CreatedAt:    time.Now().UTC().Format(time.RFC3339),
	}
	record.ID = "EVD-" + record.Key()[:12]
	return record, nil
}

func writeRecord(wt *gitx.Repo, doc *ChangeDoc, record *evidence.Record) error {
	data, err := json.MarshalIndent(record, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	dir := filepath.Join(wt.WorkDir, filepath.FromSlash(changeDir(doc.ID)), "evidence")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(dir, evidence.FileName(record.Key())), data, 0o644)
}

// loadRecordsAt collects every retained evidence record under changes/ in a
// revision — the reuse pool spans the whole promoted history plus the
// current candidate.
func loadRecordsAt(repo *gitx.Repo, rev string) ([]*evidence.Record, error) {
	files, err := repo.LsTree(rev)
	if err != nil {
		return nil, err
	}
	var records []*evidence.Record
	for path, oid := range files {
		if !strings.HasPrefix(path, "changes/") || !strings.Contains(path, "/evidence/EVD-") || !strings.HasSuffix(path, ".json") {
			continue
		}
		content, err := repo.CatBlob(oid)
		if err != nil {
			return nil, err
		}
		var record evidence.Record
		if json.Unmarshal(content, &record) == nil && record.Schema == 1 {
			records = append(records, &record)
		}
	}
	return records, nil
}
