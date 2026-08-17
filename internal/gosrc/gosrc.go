// Package gosrc derives code nodes and edges from Go sources with the
// standard parser only: packages, files, top-level symbols (declared_in),
// and package-level import edges. Everything it produces is authority
// "derived" — an analysis projection, never normative. Call-graph edges are
// deliberately out of scope for 0.6 (they would be authority "candidate").
package gosrc

import (
	"go/ast"
	"go/parser"
	"go/token"
	"path"
	"sort"
	"strings"
)

// Symbol is one top-level declaration.
type Symbol struct {
	Package   string // repo-relative package dir ("." for root)
	File      string // repo-relative file path
	Name      string // Recv.Name for methods
	Kind      string // func|method|type|const|var
	Exported  bool
	StartLine int
	EndLine   int
}

// ID is the graph node id of the symbol: "sym:<pkg>.<name>".
func (s Symbol) ID() string { return "sym:" + s.Package + "." + s.Name }

// Import is one package→package import edge (repo-internal only).
type Import struct {
	From string // repo-relative package dir
	To   string // repo-relative package dir
}

// Analysis is the derived code model of a tree.
type Analysis struct {
	Packages []string
	Symbols  []Symbol
	Imports  []Import
}

// Analyze parses every .go file (path → content, slash-separated paths).
// modulePath is the module identity from go.mod (used to resolve internal
// imports); empty disables import-edge derivation. Files that fail to parse
// are skipped — analysis is best-effort by design.
func Analyze(files map[string][]byte, modulePath string) Analysis {
	var analysis Analysis
	packages := map[string]bool{}
	importsSeen := map[Import]bool{}

	paths := make([]string, 0, len(files))
	for p := range files {
		if strings.HasSuffix(p, ".go") {
			paths = append(paths, p)
		}
	}
	sort.Strings(paths)

	for _, filePath := range paths {
		fset := token.NewFileSet()
		parsed, err := parser.ParseFile(fset, filePath, files[filePath], 0)
		if err != nil {
			continue
		}
		pkgDir := path.Dir(filePath)
		if pkgDir == "" {
			pkgDir = "."
		}
		packages[pkgDir] = true

		for _, decl := range parsed.Decls {
			switch d := decl.(type) {
			case *ast.FuncDecl:
				sym := Symbol{
					Package: pkgDir, File: filePath, Name: d.Name.Name, Kind: "func",
					Exported:  d.Name.IsExported(),
					StartLine: fset.Position(d.Pos()).Line, EndLine: fset.Position(d.End()).Line,
				}
				if d.Recv != nil && len(d.Recv.List) == 1 {
					sym.Name = receiverName(d.Recv.List[0].Type) + "." + d.Name.Name
					sym.Kind = "method"
				}
				analysis.Symbols = append(analysis.Symbols, sym)
			case *ast.GenDecl:
				kind := ""
				switch d.Tok {
				case token.TYPE:
					kind = "type"
				case token.CONST:
					kind = "const"
				case token.VAR:
					kind = "var"
				default:
					continue
				}
				for _, spec := range d.Specs {
					switch s := spec.(type) {
					case *ast.TypeSpec:
						analysis.Symbols = append(analysis.Symbols, Symbol{
							Package: pkgDir, File: filePath, Name: s.Name.Name, Kind: kind,
							Exported:  s.Name.IsExported(),
							StartLine: fset.Position(s.Pos()).Line, EndLine: fset.Position(s.End()).Line,
						})
					case *ast.ValueSpec:
						for _, n := range s.Names {
							if n.Name == "_" {
								continue
							}
							analysis.Symbols = append(analysis.Symbols, Symbol{
								Package: pkgDir, File: filePath, Name: n.Name, Kind: kind,
								Exported:  n.IsExported(),
								StartLine: fset.Position(s.Pos()).Line, EndLine: fset.Position(s.End()).Line,
							})
						}
					}
				}
			}
		}

		if modulePath != "" {
			for _, imp := range parsed.Imports {
				target := strings.Trim(imp.Path.Value, `"`)
				rel, ok := strings.CutPrefix(target, modulePath)
				if !ok {
					continue
				}
				rel = strings.TrimPrefix(rel, "/")
				if rel == "" {
					rel = "."
				}
				edge := Import{From: pkgDir, To: rel}
				if edge.From != edge.To && !importsSeen[edge] {
					importsSeen[edge] = true
					analysis.Imports = append(analysis.Imports, edge)
				}
			}
		}
	}

	for pkg := range packages {
		analysis.Packages = append(analysis.Packages, pkg)
	}
	sort.Strings(analysis.Packages)
	sort.Slice(analysis.Imports, func(i, j int) bool {
		if analysis.Imports[i].From != analysis.Imports[j].From {
			return analysis.Imports[i].From < analysis.Imports[j].From
		}
		return analysis.Imports[i].To < analysis.Imports[j].To
	})
	return analysis
}

// ModulePath extracts the module path from go.mod content.
func ModulePath(gomod []byte) string {
	for _, line := range strings.Split(string(gomod), "\n") {
		if rest, ok := strings.CutPrefix(strings.TrimSpace(line), "module "); ok {
			return strings.TrimSpace(rest)
		}
	}
	return ""
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
