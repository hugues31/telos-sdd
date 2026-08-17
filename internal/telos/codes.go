package telos

// Code is one structured error code of the V2 surface. This registry is the
// single source of truth: the agent protocol table in the bundle is generated
// from it, and CI cross-checks that every code referenced by agent assets
// exists here (and vice versa).
type Code struct {
	Name        string
	AgentAction string
}

// Codes lists every TELOS_* error code of the v0.6 surface with the action an
// agent should take when it receives it. Ordering groups codes by concern:
// generic, repository state, contract, evidence, change lifecycle, knowledge
// layer, policy/constraints, view.
var Codes = []Code{
	// Generic (kept from V1).
	{"TELOS_COMMAND_FAILED", "Unexpected failure; read the message, retry once, then report to the human."},
	{"TELOS_INPUT_INVALID", "The arguments or payload are malformed; fix the invocation, never work around the broker."},
	{"TELOS_INPUT_REQUIRED", "A required flag or stdin payload is missing; supply it."},
	{"TELOS_CONFIG_INVALID", "telos.toml is invalid; report to the human — configuration is human-owned."},
	{"TELOS_GIT_UNAVAILABLE", "Git is not installed or not on PATH; report to the human."},
	{"TELOS_GIT_REPOSITORY_REQUIRED", "Run inside a Git worktree; telos init requires one."},
	{"TELOS_NOT_INITIALIZED", "No Telos project here; telos init is a human decision."},

	// Repository state.
	{"TELOS_STATE_CORRUPTED", "The certified worktree diverged; present the salvage proposal from telos status — capture the diff as a Change, or restore."},
	{"TELOS_CERTIFICATE_INVALID", "HEAD carries no valid certificate (missing, forged, or bound to another commit); report to the human and route through salvage or restore."},

	// Contract.
	{"TELOS_CONTRACT_INVALID", "The contract or delta is structurally invalid; fix the named problems in contract.delta.md."},
	{"TELOS_CONTRACT_TAMPERED", "spec/ was edited directly in the candidate; contract semantics go through contract.delta.md — revert the direct edit and use the delta."},
	{"TELOS_REQUIREMENT_UNKNOWN", "A cited REQ id does not exist in the target contract; fix the citation or the delta."},
	{"TELOS_APPROVAL_REQUIRED", "The transition needs a human approval that is not recorded; run telos change review and present the exact delta."},
	{"TELOS_APPROVAL_STALE", "Content changed since review; run telos change review again and re-present — approvals bind to exact bytes."},
	{"TELOS_NOTHING_PENDING", "No pending delta to review; nothing to do."},

	// Evidence.
	{"TELOS_TESTS_FAILED", "The suite is red; fix the implementation in the candidate, never weaken tests."},
	{"TELOS_TEST_FIRST", "Unproven requirements are implemented test-first; submit the failing test and witness red before any implementation."},
	{"TELOS_TEST_SEALED", "Sealed red tests may change only through a new red witness; fix the implementation instead."},
	{"TELOS_RED_EXPECTED", "The new test already passes, so it proves nothing; strengthen it, or offer evidence adopt (human-gated) for already-correct behavior."},
	{"TELOS_RED_PENDING", "A witnessed red awaits its green; implement until the sealed tests pass untouched."},
	{"TELOS_RED_STALE", "Sealed test bytes changed; re-witness from red — the seal is never edited to fit."},
	{"TELOS_BASELINE_RED", "A new test is only evidence on a green baseline; make the suite pass first — one cycle at a time."},
	{"TELOS_OBLIGATION_UNMET", "A requirement lacks its required evidence; produce it (witnessed red/green or the kinds its class's policy demands)."},
	{"TELOS_FINDING_BLOCKING", "Open blocking findings forbid certification; fix the underlying issue or have the human resolve them."},

	// Change lifecycle.
	{"TELOS_BASE_STALE", "main moved since this Change's base; run telos change rebase, then retry."},
	{"TELOS_CHANGE_UNKNOWN", "No such Change; list open changes via telos status."},
	{"TELOS_CHANGE_STATE_INVALID", "This verb is not valid in the Change's current status; follow next_actions."},
	{"TELOS_CANDIDATE_REQUIRED", "Run this command inside the Change's candidate worktree."},
	{"TELOS_ROOT_REQUIRED", "Run this command in the certified root worktree, not a candidate."},
	{"TELOS_WORKTREE_CONFLICT", "A salvage or rebase hit conflicts; resolve them in the named worktree — preserved work is never dropped."},

	// Knowledge layer.
	{"TELOS_INDEX_STALE", "The derived index does not match the current tree; run telos index rebuild."},
	{"TELOS_NODE_NOT_FOUND", "No node with that id; use telos search to locate it."},
	{"TELOS_SYMBOL_AMBIGUOUS", "Several symbols match; requalify using one of the listed candidates."},
	{"TELOS_BUDGET_TOO_SMALL", "The context budget cannot fit the global invariants; retry with at least the stated minimum."},

	// Policy and constraints.
	{"TELOS_POLICY_INVALID", "policies/ does not compile; report to the human — policy is privileged content."},
	{"TELOS_POLICY_WEAKENS_KERNEL", "Project policy conflicts with a kernel floor; kernel invariants cannot be weakened — remove the offending rule."},
	{"TELOS_CONSTRAINT_UNSAT", "Formalized requirements are provably contradictory; a human must resolve the named REQs."},

	// View.
	{"TELOS_PORT_BUSY", "The view port is taken; pass --port (0 for an ephemeral port)."},
}
