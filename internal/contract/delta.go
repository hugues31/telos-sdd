package contract

import (
	"fmt"
	"regexp"
	"strings"
)

// A Change's contract delta (changes/CHG-NNN/contract.delta.md) is a list of
// operations over the canonical contract, each introduced by a marker:
//
//	<!-- telos:op add file: spec/auth.md -->
//	### REQ-007 — Sessions expire
//	...
//
//	<!-- telos:op replace file: spec/auth.md -->
//	### REQ-003 — ...
//
//	<!-- telos:op remove id: REQ-004 -->
//
// Folding the delta over the base contract is a pure function; the folded
// spec/ tree OID is the approval digest (KERNEL-004).

// OpKind is a delta operation kind.
type OpKind string

const (
	OpAdd     OpKind = "add"
	OpReplace OpKind = "replace"
	OpRemove  OpKind = "remove"
)

// Op is one delta operation. For add/replace, Section carries the full
// section text and ID its heading id; for remove only ID is set.
type Op struct {
	Kind    OpKind
	File    string
	ID      string
	Section string
}

var (
	opMarker     = regexp.MustCompile(`(?m)^<!--\s*telos:op\s+(add|replace|remove)\s+(file|id):\s*(\S+)\s*-->\s*$`)
	anyIDHeading = regexp.MustCompile(`(?m)^###\s+((?:INT|REQ|DEC)-[0-9]{3,})\b`)
)

// ParseDelta parses a contract delta document into operations. An empty
// document yields no operations.
func ParseDelta(data []byte) ([]Op, error) {
	body := string(normalize(data))
	markers := opMarker.FindAllStringSubmatchIndex(body, -1)
	if len(markers) == 0 {
		if strings.TrimSpace(stripDeltaComments(body)) != "" {
			return nil, fmt.Errorf("contract.delta.md has content outside any telos:op marker")
		}
		return nil, nil
	}
	if before := body[:markers[0][0]]; strings.TrimSpace(stripDeltaComments(before)) != "" {
		return nil, fmt.Errorf("contract.delta.md has content before the first telos:op marker")
	}
	var ops []Op
	for i, m := range markers {
		kind := OpKind(body[m[2]:m[3]])
		attr := body[m[4]:m[5]]
		value := body[m[6]:m[7]]
		end := len(body)
		if i+1 < len(markers) {
			end = markers[i+1][0]
		}
		section := strings.TrimSpace(body[m[1]:end])
		op := Op{Kind: kind}
		switch kind {
		case OpAdd, OpReplace:
			if attr != "file" {
				return nil, fmt.Errorf("telos:op %s takes `file:`, got `%s:`", kind, attr)
			}
			if !strings.HasPrefix(value, Dir+"/") || !strings.HasSuffix(value, ".md") {
				return nil, fmt.Errorf("telos:op %s targets %q; the target must be a Markdown file under %s/", kind, value, Dir)
			}
			ids := anyIDHeading.FindAllStringSubmatch(section, -1)
			if len(ids) != 1 {
				return nil, fmt.Errorf("telos:op %s %s must carry exactly one `### <ID> — Title` section, found %d", kind, value, len(ids))
			}
			op.File = value
			op.ID = ids[0][1]
			op.Section = section
		case OpRemove:
			if attr != "id" {
				return nil, fmt.Errorf("telos:op remove takes `id:`, got `%s:`", attr)
			}
			if !anyIDHeading.MatchString("### " + value + " —") {
				return nil, fmt.Errorf("telos:op remove targets %q; expected an INT/REQ/DEC id", value)
			}
			if section != "" {
				return nil, fmt.Errorf("telos:op remove id: %s carries unexpected content", value)
			}
			op.ID = value
		default:
			return nil, fmt.Errorf("unknown telos:op kind %q", kind)
		}
		ops = append(ops, op)
	}
	return ops, nil
}

// stripDeltaComments removes HTML comments that are not op markers, so a
// template full of guidance comments parses as an empty delta.
var htmlComment = regexp.MustCompile(`(?s)<!--.*?-->`)

func stripDeltaComments(s string) string {
	return htmlComment.ReplaceAllString(s, "")
}

// Fold applies the operations to a copy of the base contract files and
// returns the folded files. It is a pure function of (base, ops).
func Fold(base map[string][]byte, ops []Op) (map[string][]byte, error) {
	folded := make(map[string][]byte, len(base))
	for path, content := range base {
		folded[path] = append([]byte(nil), content...)
	}
	for _, op := range ops {
		switch op.Kind {
		case OpAdd:
			if path, _, _, ok := locateSection(folded, op.ID); ok {
				return nil, fmt.Errorf("telos:op add: %s already exists in %s; use replace", op.ID, path)
			}
			existing := string(folded[op.File])
			if existing != "" && !strings.HasSuffix(existing, "\n") {
				existing += "\n"
			}
			if existing != "" {
				existing += "\n"
			}
			folded[op.File] = []byte(existing + op.Section + "\n")
		case OpReplace:
			body, ok := folded[op.File]
			if !ok {
				return nil, fmt.Errorf("telos:op replace: %s does not exist in the base contract", op.File)
			}
			start, end, found := sectionBounds(string(body), op.ID)
			if !found {
				return nil, fmt.Errorf("telos:op replace: %s not found in %s", op.ID, op.File)
			}
			folded[op.File] = []byte(string(body)[:start] + op.Section + "\n" + string(body)[end:])
		case OpRemove:
			path, start, end, ok := locateSection(folded, op.ID)
			if !ok {
				return nil, fmt.Errorf("telos:op remove: %s not found in the base contract", op.ID)
			}
			body := string(folded[path])
			remainder := body[:start] + body[end:]
			if strings.TrimSpace(remainder) == "" {
				delete(folded, path)
			} else {
				folded[path] = []byte(remainder)
			}
		}
	}
	return folded, nil
}

// sectionBounds finds the byte range of the section with the given id inside
// one file body (heading through the next ### heading).
func sectionBounds(body, id string) (start, end int, ok bool) {
	for _, m := range anyIDHeading.FindAllStringSubmatchIndex(body, -1) {
		if body[m[2]:m[3]] != id {
			continue
		}
		end := len(body)
		if next := anyHeading.FindStringIndex(body[m[1]:]); next != nil {
			end = m[1] + next[0]
		}
		return m[0], end, true
	}
	return 0, 0, false
}

// locateSection finds the file and byte range holding the section id across
// all contract files.
func locateSection(files map[string][]byte, id string) (path string, start, end int, ok bool) {
	for p, content := range files {
		if s, e, found := sectionBounds(string(normalize(content)), id); found {
			return p, s, e, true
		}
	}
	return "", 0, 0, false
}
