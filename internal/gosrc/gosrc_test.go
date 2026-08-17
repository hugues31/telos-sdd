package gosrc

import "testing"

func TestAnalyze(t *testing.T) {
	files := map[string][]byte{
		"go.mod": []byte("module example.com/toy\n\ngo 1.24\n"),
		"pkga/a.go": []byte(`package pkga

import "example.com/toy/pkgb"

type Service struct{}

func (s *Service) Login() error { return pkgb.Check() }

func helper() int { return 1 }

const Limit = 10
`),
		"pkgb/b.go":      []byte("package pkgb\n\nfunc Check() error { return nil }\n"),
		"pkga/broken.go": []byte("not go {"),
	}
	analysis := Analyze(files, ModulePath(files["go.mod"]))

	if len(analysis.Packages) != 2 {
		t.Fatalf("packages = %v", analysis.Packages)
	}
	byID := map[string]Symbol{}
	for _, s := range analysis.Symbols {
		byID[s.ID()] = s
	}
	login, ok := byID["sym:pkga.Service.Login"]
	if !ok || login.Kind != "method" || !login.Exported || login.StartLine == 0 {
		t.Fatalf("Login = %+v (all: %v)", login, analysis.Symbols)
	}
	if h, ok := byID["sym:pkga.helper"]; !ok || h.Exported {
		t.Fatalf("helper = %+v", h)
	}
	if c, ok := byID["sym:pkga.Limit"]; !ok || c.Kind != "const" {
		t.Fatalf("Limit = %+v", c)
	}
	if len(analysis.Imports) != 1 || analysis.Imports[0] != (Import{From: "pkga", To: "pkgb"}) {
		t.Fatalf("imports = %v", analysis.Imports)
	}
}

func TestModulePath(t *testing.T) {
	if got := ModulePath([]byte("// comment\nmodule github.com/x/y\n\ngo 1.24\n")); got != "github.com/x/y" {
		t.Fatalf("ModulePath = %q", got)
	}
	if got := ModulePath(nil); got != "" {
		t.Fatalf("empty go.mod = %q", got)
	}
}
