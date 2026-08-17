// Package provenance records, at promotion time, how a Change connects
// requirements to code and tests: REQ → implemented_by (symbols or files) /
// verified_by (citing tests) / changed_by (the Change). It replaces V1's
// source annotations entirely. Durable identity is REQ → Change → Evidence;
// symbol names are derived projections that renames cannot break.
package provenance

import (
	"go/ast"
	"go/parser"
	"go/token"
	"sort"
	"strings"
)

// Authorities and origins.
const (
	AuthorityCanonical = "canonical"
	AuthorityDerived   = "derived"

	OriginGoAST    = "go_ast"
	OriginFileDiff = "file_diff"
)

// Relation is one provenance edge.
type Relation struct {
	Req       string `json:"req"`
	Rel       string `json:"rel"` // implemented_by|verified_by|changed_by
	Symbol    string `json:"symbol,omitempty"`
	Path      string `json:"path,omitempty"`
	Evidence  string `json:"evidence,omitempty"`
	Authority string `json:"authority"`
	Origin    string `json:"origin"`
}

// Doc is changes/CHG-NNN/provenance.json.
type Doc struct {
	Schema    int        `json:"provenance"`
	Change    string     `json:"change"`
	Relations []Relation `json:"relations"`
}

// symbolsOf maps every top-level declaration of a Go source file to its
// source text. Parse failures yield nil (the caller falls back to
// file-level provenance).
func symbolsOf(src []byte) map[string]string {
	fset := token.NewFileSet()
	file, err := parser.ParseFile(fset, "src.go", src, 0)
	if err != nil {
		return nil
	}
	text := func(node ast.Node) string {
		start := fset.Position(node.Pos()).Offset
		end := fset.Position(node.End()).Offset
		if start < 0 || end > len(src) || start >= end {
			return ""
		}
		return string(src[start:end])
	}
	out := map[string]string{}
	for _, decl := range file.Decls {
		switch d := decl.(type) {
		case *ast.FuncDecl:
			name := d.Name.Name
			if d.Recv != nil && len(d.Recv.List) == 1 {
				name = receiverName(d.Recv.List[0].Type) + "." + name
			}
			out[name] = text(d)
		case *ast.GenDecl:
			for _, spec := range d.Specs {
				switch s := spec.(type) {
				case *ast.TypeSpec:
					out[s.Name.Name] = text(s)
				case *ast.ValueSpec:
					for _, n := range s.Names {
						out[n.Name] = text(s)
					}
				}
			}
		}
	}
	return out
}

func receiverName(expr ast.Expr) string {
	switch t := expr.(type) {
	case *ast.StarExpr:
		return receiverName(t.X)
	case *ast.Ident:
		return t.Name
	case *ast.IndexExpr:
		return receiverName(t.X)
	case *ast.IndexListExpr:
		return receiverName(t.X)
	}
	return "?"
}

// ChangedSymbols lists the top-level Go symbols whose declaration text is
// new or different between the base and head versions of one file. A nil
// result means the file could not be analyzed (fall back to file level).
func ChangedSymbols(base, head []byte) []string {
	headSyms := symbolsOf(head)
	if headSyms == nil {
		return nil
	}
	baseSyms := symbolsOf(base) // nil for a new or unparsable base: every head symbol counts
	var out []string
	for name, text := range headSyms {
		if baseSyms == nil || baseSyms[name] != text {
			out = append(out, name)
		}
	}
	sort.Strings(out)
	return out
}

// FileVersions holds the base and head contents of one changed file.
type FileVersions struct {
	Base []byte
	Head []byte
}

// Build assembles the provenance document for a promotion: for every proven
// requirement, its verifying tests (canonical — they come from witnessed
// evidence) and the changed implementation, symbol-level where Go analysis
// succeeds, file-level otherwise (derived either way).
func Build(changeID string, provenReqs []string, changedCode map[string]FileVersions, verifiedBy map[string][]string, evidenceIDs map[string]string) Doc {
	doc := Doc{Schema: 1, Change: changeID, Relations: []Relation{}}
	reqs := append([]string(nil), provenReqs...)
	sort.Strings(reqs)

	type implEntry struct {
		symbol, path, origin string
	}
	var impls []implEntry
	paths := make([]string, 0, len(changedCode))
	for p := range changedCode {
		paths = append(paths, p)
	}
	sort.Strings(paths)
	for _, p := range paths {
		v := changedCode[p]
		if strings.HasSuffix(p, ".go") {
			if symbols := ChangedSymbols(v.Base, v.Head); symbols != nil {
				for _, s := range symbols {
					impls = append(impls, implEntry{symbol: s, path: p, origin: OriginGoAST})
				}
				continue
			}
		}
		impls = append(impls, implEntry{path: p, origin: OriginFileDiff})
	}

	for _, req := range reqs {
		doc.Relations = append(doc.Relations, Relation{
			Req: req, Rel: "changed_by", Authority: AuthorityCanonical, Origin: changeID,
		})
		tests := append([]string(nil), verifiedBy[req]...)
		sort.Strings(tests)
		for _, test := range tests {
			doc.Relations = append(doc.Relations, Relation{
				Req: req, Rel: "verified_by", Path: test, Evidence: evidenceIDs[req],
				Authority: AuthorityCanonical, Origin: changeID,
			})
		}
		for _, impl := range impls {
			doc.Relations = append(doc.Relations, Relation{
				Req: req, Rel: "implemented_by", Symbol: impl.symbol, Path: impl.path,
				Authority: AuthorityDerived, Origin: impl.origin,
			})
		}
	}
	return doc
}
