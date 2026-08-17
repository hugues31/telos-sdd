package main

import (
	"fmt"
	"strings"
	"testing"
)

// TestCLIKnowledge is the M5 oracle: after a promoted change, the derived
// graph answers search/show/related/impact from certified artifacts alone.
func TestCLIKnowledge(t *testing.T) {
	bin := buildCLI(t)
	root := setupCertified(t, bin)

	// A condensed loop 1: promote REQ-001.
	id, wt := startChange(t, bin, root, "behavior_change", "Emit the greeting")
	write(t, wt, "changes/"+id+"/contract.delta.md", v2GreetingDelta)
	approveChange(t, bin, wt)
	write(t, wt, "tests/core_test.txt", "asserts REQ-001\nexpect out/greeting.txt\n")
	expectOK(t, runCLI(t, bin, wt, "", "evidence", "red", "--req", "REQ-001"), "red")
	write(t, wt, "out/greeting.txt", "greeting\n")
	expectOK(t, runCLI(t, bin, wt, "", "evidence", "green", "--req", "REQ-001"), "green")
	expectOK(t, runCLI(t, bin, wt, "", "change", "promote"), "promote")

	// The index rebuilds from the certified tree and serves the graph.
	status := expectOK(t, runCLI(t, bin, root, "", "index", "status"), "index status")
	if status["nodes"] == nil {
		t.Fatalf("index status = %v", status)
	}

	search := expectOK(t, runCLI(t, bin, root, "", "search", "greeting"), "search")
	if !strings.Contains(fmt.Sprint(search["hits"]), "REQ-001") {
		t.Fatalf("search = %v", search)
	}

	show := expectOK(t, runCLI(t, bin, root, "", "show", "REQ-001"), "show")
	node, _ := show["node"].(map[string]any)
	if node == nil || node["kind"] != "requirement" {
		t.Fatalf("show = %v", show)
	}
	if show["evidence"] == nil {
		t.Fatalf("show misses evidence: %v", show)
	}

	related := expectOK(t, runCLI(t, bin, root, "", "related", "REQ-001", "--depth", "2"), "related")
	if !strings.Contains(fmt.Sprint(related["nodes"]), "INT-001") {
		t.Fatalf("related = %v", related)
	}

	impact := expectOK(t, runCLI(t, bin, root, "", "impact", "INT-001"), "impact")
	if !strings.Contains(fmt.Sprint(impact["impact"]), "REQ-001") {
		t.Fatalf("impact = %v", impact)
	}

	expectCode(t, runCLI(t, bin, root, "", "show", "REQ-404"), "TELOS_NODE_NOT_FOUND", "show unknown")
	expectCode(t, runCLI(t, bin, root, "", "explain", "NoSuchSymbol"), "TELOS_NODE_NOT_FOUND", "explain unknown")
}
