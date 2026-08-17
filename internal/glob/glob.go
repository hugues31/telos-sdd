// Package glob matches slash-separated relative paths against the pattern
// dialect used across Telos configuration (`test_files`, policy path rules):
// `*` and `?` stay within one path segment; `**` spans any number of
// segments, including zero.
package glob

import (
	"path"
	"strings"
)

// Match reports whether the slash-separated relative path rel matches pattern.
func Match(pattern, rel string) bool {
	return matchSegments(strings.Split(pattern, "/"), strings.Split(rel, "/"))
}

// MatchAny reports whether rel matches at least one of the patterns.
func MatchAny(patterns []string, rel string) bool {
	for _, pattern := range patterns {
		if Match(pattern, rel) {
			return true
		}
	}
	return false
}

func matchSegments(pattern, segments []string) bool {
	if len(pattern) == 0 {
		return len(segments) == 0
	}
	if pattern[0] == "**" {
		if matchSegments(pattern[1:], segments) {
			return true
		}
		return len(segments) > 0 && matchSegments(pattern, segments[1:])
	}
	if len(segments) == 0 {
		return false
	}
	if ok, err := path.Match(pattern[0], segments[0]); err != nil || !ok {
		return false
	}
	return matchSegments(pattern[1:], segments[1:])
}
