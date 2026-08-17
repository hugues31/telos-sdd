package telos

import (
	"fmt"
	"io"
	"strings"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/gitx"
	"github.com/hugues31/telos-sdd/internal/kernel"
)

func runEvidence(repo *gitx.Repo, args []string, stdout, stderr io.Writer) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "evidence requires a verb: red, green, adopt, or mutation")
	}
	verb := args[0]
	cfg, err := kernel.ReadConfig(repo.WorkDir)
	if err != nil {
		return commandExecution{}, err
	}
	if verb == "mutation" {
		_, record, outcome, err := kernel.EvidenceMutation(repo, cfg, stdout)
		if err != nil {
			return commandExecution{}, err
		}
		human := fmt.Sprintf("Mutation run: %d site(s), %d killed, %d survived (score %.2f).", outcome.Sites, outcome.Killed, outcome.Survived, outcome.Score)
		if outcome.Survived > 0 {
			human += "\nSurvivors mean the tests cannot tell the mutant from the real program — triage them as findings."
		}
		return commandExecution{Command: "evidence.mutation", Result: map[string]any{"record": record, "outcome": outcome}, Human: human}, nil
	}
	f := flags("evidence "+verb, stderr)
	req := f.String("req", "", "REQ-NNN the evidence proves")
	if err := f.Parse(args[1:]); err != nil {
		return commandExecution{}, err
	}
	if *req == "" {
		return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "evidence "+verb+" requires --req REQ-NNN")
	}
	reqID := strings.ToUpper(*req)
	switch verb {
	case "red":
		_, witness, err := kernel.EvidenceRed(repo, cfg, reqID, stdout)
		if err != nil {
			return commandExecution{}, err
		}
		paths := make([]string, len(witness.SealedTests))
		for i, s := range witness.SealedTests {
			paths[i] = s.Path
		}
		human := fmt.Sprintf("Red witnessed for %s: %s sealed; only the implementation may turn them green.", reqID, strings.Join(paths, ", "))
		return commandExecution{Command: "evidence.red", Result: map[string]any{"req": reqID, "sealed_tests": witness.SealedTests, "suite": "red"}, Next: []string{"evidence green"}, Human: human}, nil
	case "green":
		_, record, err := kernel.EvidenceGreen(repo, cfg, reqID, stdout)
		if err != nil {
			return commandExecution{}, err
		}
		human := fmt.Sprintf("Green witnessed for %s: sealed tests passed untouched; evidence %s recorded.", reqID, record.ID)
		return commandExecution{Command: "evidence.green", Result: map[string]any{"req": reqID, "record": record, "suite": "green"}, Next: []string{"change ready"}, Human: human}, nil
	case "adopt":
		_, record, err := kernel.EvidenceAdopt(repo, cfg, reqID, stdout)
		if err != nil {
			return commandExecution{}, err
		}
		human := fmt.Sprintf("Adopted existing behavior as proof of %s (evidence %s).", reqID, record.ID)
		return commandExecution{Command: "evidence.adopt", Result: map[string]any{"req": reqID, "record": record}, Next: []string{"change ready"}, Human: human}, nil
	default:
		return commandExecution{}, coded.New("TELOS_INPUT_INVALID", fmt.Sprintf("unknown evidence verb %q", verb))
	}
}

func runFindings(repo *gitx.Repo, args []string, stderr io.Writer) (commandExecution, error) {
	if len(args) == 0 {
		return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "findings requires a verb: list, add, confirm, or resolve")
	}
	switch args[0] {
	case "list":
		doc, err := kernel.LoadChange(repo)
		if err != nil {
			return commandExecution{}, err
		}
		findings, err := kernel.LoadFindings(repo, doc.ID)
		if err != nil {
			return commandExecution{}, err
		}
		open, blocking := 0, 0
		for _, f := range findings {
			if f.Status == kernel.FindingOpen {
				open++
				if f.Severity == kernel.SeverityBlocking {
					blocking++
				}
			}
		}
		result := map[string]any{"findings": findings, "counts": map[string]int{"total": len(findings), "open": open, "blocking": blocking}}
		return commandExecution{Command: "findings.list", Result: result}, nil
	case "add":
		f := flags("findings add", stderr)
		severity := f.String("severity", "", "proposed severity: info, minor, major, blocking")
		rationale := f.String("rationale", "", "why this finding matters")
		confidence := f.Float64("confidence", 0, "critic confidence 0..1")
		critic := f.String("critic", "", "critic name; omitted means a human finding")
		reqs := f.String("req", "", "comma-separated REQ ids the finding targets")
		if err := f.Parse(args[1:]); err != nil {
			return commandExecution{}, err
		}
		source := kernel.FindingSource{Kind: "human", Name: "human"}
		if *critic != "" {
			source = kernel.FindingSource{Kind: "critic", Name: *critic}
		}
		var target kernel.FindingTarget
		if *reqs != "" {
			for _, r := range strings.Split(*reqs, ",") {
				target.Requirements = append(target.Requirements, strings.ToUpper(strings.TrimSpace(r)))
			}
		}
		finding, err := kernel.AddFinding(repo, kernel.Finding{
			Source: source, Target: target,
			ProposedSeverity: *severity, Confidence: *confidence, Rationale: *rationale,
		})
		if err != nil {
			return commandExecution{}, err
		}
		human := fmt.Sprintf("%s recorded (%s proposes %s).", finding.ID, source.Name, finding.ProposedSeverity)
		return commandExecution{Command: "findings.add", Result: map[string]any{"finding": finding}, Human: human}, nil
	case "confirm":
		if len(args) < 2 {
			return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "findings confirm takes the FND-NNN id")
		}
		finding, err := kernel.ConfirmFinding(repo, strings.ToUpper(args[1]))
		if err != nil {
			return commandExecution{}, err
		}
		return commandExecution{Command: "findings.confirm", Result: map[string]any{"finding": finding}, Human: finding.ID + " confirmed as " + finding.Severity + "."}, nil
	case "resolve":
		if len(args) < 2 {
			return commandExecution{}, coded.New("TELOS_INPUT_REQUIRED", "findings resolve takes the FND-NNN id")
		}
		id := strings.ToUpper(args[1])
		f := flags("findings resolve", stderr)
		as := f.String("as", "", "real, not_an_issue, or duplicate")
		of := f.String("of", "", "FND-NNN this duplicates")
		note := f.String("note", "", "resolution note")
		if err := f.Parse(args[2:]); err != nil {
			return commandExecution{}, err
		}
		finding, err := kernel.ResolveFinding(repo, id, *as, strings.ToUpper(*of), *note)
		if err != nil {
			return commandExecution{}, err
		}
		return commandExecution{Command: "findings.resolve", Result: map[string]any{"finding": finding}, Human: finding.ID + " resolved as " + finding.Resolution.Kind + "."}, nil
	default:
		return commandExecution{}, coded.New("TELOS_INPUT_INVALID", fmt.Sprintf("unknown findings verb %q", args[0]))
	}
}
