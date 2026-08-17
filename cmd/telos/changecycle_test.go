package main

import (
	"fmt"
	"strings"
	"testing"
)

// TestCLIChangeLifecycle is the M2 oracle: loop 1 through approval — an
// isolated candidate, a contract delta folded over the base, and an approval
// bound to the exact reviewed digest.
func TestCLIChangeLifecycle(t *testing.T) {
	bin := buildCLI(t)
	root := setupCertified(t, bin)

	// Start a behavior change; the candidate is a sibling worktree.
	started := expectOK(t, runCLI(t, bin, root, "", "change", "start", "--category", "behavior_change", "--title", "Emit the greeting"), "change start")
	id, _ := started["id"].(string)
	wt, _ := started["worktree"].(string)
	base, _ := started["base"].(string)
	if id != "CHG-001" || wt == "" || base == "" {
		t.Fatalf("change start = %v", started)
	}

	// Root status lists the open change; candidate status names it.
	status := expectOK(t, runCLI(t, bin, root, "", "status"), "root status")
	changes, _ := status["changes"].([]any)
	if len(changes) != 1 {
		t.Fatalf("root status changes = %v", status)
	}
	status = expectOK(t, runCLI(t, bin, wt, "", "status"), "candidate status")
	if status["context"] != "candidate" {
		t.Fatalf("candidate status = %v", status)
	}
	change, _ := status["change"].(map[string]any)
	if change == nil || change["id"] != id || change["status"] != "drafting" {
		t.Fatalf("candidate change = %v", change)
	}

	// The scaffold template is an empty delta.
	expectCode(t, runCLI(t, bin, wt, "", "change", "review"), "TELOS_NOTHING_PENDING", "review empty delta")

	// Review computes the folded-contract digest.
	write(t, wt, "changes/"+id+"/contract.delta.md", v2GreetingDelta)
	review := expectOK(t, runCLI(t, bin, wt, "", "change", "review"), "review")
	digest, _ := review["digest"].(string)
	if digest == "" || review["kind"] != "contract" {
		t.Fatalf("review = %v", review)
	}

	// Drift invalidates the presented digest.
	write(t, wt, "changes/"+id+"/contract.delta.md", v2GreetingDelta+"\nLate drift makes this delta invalid prose.\n")
	expectCode(t, runCLI(t, bin, wt, "", "change", "approve", "--digest", digest), "TELOS_APPROVAL_STALE", "stale approve")

	// Back to the reviewed content: the digest is deterministic and approval
	// binds to it.
	write(t, wt, "changes/"+id+"/contract.delta.md", v2GreetingDelta)
	review = expectOK(t, runCLI(t, bin, wt, "", "change", "review"), "re-review")
	if review["digest"] != digest {
		t.Fatalf("digest not deterministic: %v vs %s", review["digest"], digest)
	}
	approved := expectOK(t, runCLI(t, bin, wt, "", "change", "approve", "--digest", digest), "approve")
	doc, _ := approved["change"].(map[string]any)
	if doc == nil || doc["status"] != "approved" {
		t.Fatalf("approved = %v", approved)
	}

	// show/diff expose the candidate's content.
	show := expectOK(t, runCLI(t, bin, wt, "", "change", "show"), "show")
	if !strings.Contains(fmt.Sprint(show["changed_paths"]), "contract.delta.md") {
		t.Fatalf("show = %v", show)
	}

	// A direct spec edit in the candidate is tampering; restoring the base
	// bytes clears it.
	write(t, wt, "spec/PRODUCT.md", "smuggled\n")
	expectCode(t, runCLI(t, bin, wt, "", "change", "review"), "TELOS_CONTRACT_TAMPERED", "review after spec edit")
	git(t, wt, "checkout", base, "--", "spec/")
	review = expectOK(t, runCLI(t, bin, wt, "", "change", "review"), "review after restore")
	if review["digest"] != digest {
		t.Fatalf("digest after restore = %v", review["digest"])
	}

	// Abort is root-scoped and destructive.
	second := expectOK(t, runCLI(t, bin, root, "", "change", "start", "--category", "behavior_preserving", "--title", "cleanup"), "second change")
	expectCode(t, runCLI(t, bin, wt, "", "change", "abort", fmt.Sprint(second["id"])), "TELOS_ROOT_REQUIRED", "abort from candidate")
	expectOK(t, runCLI(t, bin, root, "", "change", "abort", fmt.Sprint(second["id"])), "abort")
	expectCode(t, runCLI(t, bin, root, "", "change", "abort", "CHG-404"), "TELOS_CHANGE_UNKNOWN", "abort unknown")

	// The certified root never moved through any of this.
	expectOK(t, runCLI(t, bin, root, "", "verify"), "verify root")
}
