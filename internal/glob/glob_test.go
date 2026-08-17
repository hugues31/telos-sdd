package glob

import "testing"

func TestMatch(t *testing.T) {
	cases := []struct {
		pattern, rel string
		want         bool
	}{
		{"tests/**", "tests/core_test.txt", true},
		{"tests/**", "tests/sub/deep_test.txt", true},
		{"tests/**", "tests", true}, // `**` spans zero segments by design
		{"**", "anything/at/all", true},
		{"**/*_test.go", "pkga/pkga_test.go", true},
		{"**/*_test.go", "a_test.go", true},
		{"**/*_test.go", "pkga/pkga.go", false},
		{"*.md", "README.md", true},
		{"*.md", "docs/README.md", false},
		{"docs/*.md", "docs/README.md", true},
		{"docs/?.md", "docs/a.md", true},
		{"docs/?.md", "docs/ab.md", false},
		{"a/**/z", "a/z", true},
		{"a/**/z", "a/b/c/z", true},
		{"a/**/z", "a/b/c", false},
		{"[", "anything", false},
	}
	for _, c := range cases {
		if got := Match(c.pattern, c.rel); got != c.want {
			t.Errorf("Match(%q, %q) = %v, want %v", c.pattern, c.rel, got, c.want)
		}
	}
}

func TestMatchAny(t *testing.T) {
	patterns := []string{"tests/**", "*.md"}
	if !MatchAny(patterns, "tests/x.txt") || !MatchAny(patterns, "README.md") {
		t.Error("MatchAny missed a matching pattern")
	}
	if MatchAny(patterns, "src/main.go") {
		t.Error("MatchAny matched a non-matching path")
	}
	if MatchAny(nil, "anything") {
		t.Error("MatchAny with no patterns must not match")
	}
}
