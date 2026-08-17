package provenance

import (
	"strings"
	"testing"
)

const baseSrc = `package auth

type Service struct{ locked bool }

func (s *Service) Login(user string) error { return nil }

func Helper() int { return 1 }
`

const headSrc = `package auth

type Service struct{ locked bool }

func (s *Service) Login(user string) error {
	if s.locked {
		return ErrLocked
	}
	return nil
}

func Helper() int { return 1 }

var ErrLocked = errorString("locked")

type errorString string

func (e errorString) Error() string { return string(e) }
`

func TestChangedSymbols(t *testing.T) {
	symbols := ChangedSymbols([]byte(baseSrc), []byte(headSrc))
	joined := strings.Join(symbols, ",")
	for _, want := range []string{"Service.Login", "ErrLocked", "errorString", "errorString.Error"} {
		if !strings.Contains(joined, want) {
			t.Errorf("changed symbols miss %s: %v", want, symbols)
		}
	}
	for _, unchanged := range []string{"Helper", "Service,"} {
		if strings.Contains(joined+",", unchanged+",") && unchanged == "Helper" {
			t.Errorf("unchanged symbol %s reported: %v", unchanged, symbols)
		}
	}
	// A new file: every symbol counts.
	if got := ChangedSymbols(nil, []byte(baseSrc)); len(got) != 3 {
		t.Errorf("new-file symbols = %v", got)
	}
	// Unparsable head falls back to nil (file-level provenance).
	if got := ChangedSymbols(nil, []byte("not go at all {")); got != nil {
		t.Errorf("unparsable head = %v, want nil", got)
	}
}

func TestBuild(t *testing.T) {
	doc := Build("CHG-007", []string{"REQ-042"},
		map[string]FileVersions{
			"internal/auth/service.go": {Base: []byte(baseSrc), Head: []byte(headSrc)},
			"assets/logo.svg":          {Head: []byte("<svg/>")},
		},
		map[string][]string{"REQ-042": {"internal/auth/service_test.go"}},
		map[string]string{"REQ-042": "EVD-abc123def456"},
	)
	var changedBy, verifiedBy, symbolImpl, fileImpl int
	for _, r := range doc.Relations {
		switch {
		case r.Rel == "changed_by" && r.Authority == AuthorityCanonical:
			changedBy++
		case r.Rel == "verified_by" && r.Evidence == "EVD-abc123def456" && r.Authority == AuthorityCanonical:
			verifiedBy++
		case r.Rel == "implemented_by" && r.Symbol != "" && r.Origin == OriginGoAST:
			symbolImpl++
		case r.Rel == "implemented_by" && r.Symbol == "" && r.Origin == OriginFileDiff:
			fileImpl++
		}
	}
	if changedBy != 1 || verifiedBy != 1 || symbolImpl == 0 || fileImpl != 1 {
		t.Fatalf("relations = %+v", doc.Relations)
	}
}
