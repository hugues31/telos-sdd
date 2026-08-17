// Package graph defines the semantic-graph domain model and the Querier
// interface shared by the CLI query commands, the context compiler, and the
// web view. It is a pure contract: no SQLite, no I/O. The only implementation
// lives in internal/index; SQL never leaks past that package.
//
// The graph is a query model, never an authority model: every row it serves
// is derived from certified repository artifacts and must be reconstructible
// from them alone (delete the index, rebuild, get the same graph).
package graph

import "time"

// NodeKind enumerates the kinds of nodes the graph holds.
type NodeKind string

const (
	KindIntent      NodeKind = "intent"
	KindRequirement NodeKind = "requirement"
	KindDecision    NodeKind = "decision"
	KindConstraint  NodeKind = "constraint"
	KindChange      NodeKind = "change"
	KindFinding     NodeKind = "finding"
	KindScenario    NodeKind = "scenario"
	KindEvidence    NodeKind = "evidence"
	KindTest        NodeKind = "test"
	KindFile        NodeKind = "file"
	KindSymbol      NodeKind = "symbol"
	KindPackage     NodeKind = "package"
	KindDomain      NodeKind = "domain"
)

// EdgeKind enumerates relation kinds. The semantic set excludes the three
// code-analysis kinds (calls, imports, uses), which traversals only include
// on request.
type EdgeKind string

const (
	EdgeMotivates    EdgeKind = "motivates"
	EdgeRefines      EdgeKind = "refines"
	EdgeSupersedes   EdgeKind = "supersedes"
	EdgeDependsOn    EdgeKind = "depends_on"
	EdgeConstrains   EdgeKind = "constrains"
	EdgeConflicts    EdgeKind = "conflicts_with"
	EdgeImplements   EdgeKind = "implements"
	EdgeVerifiedBy   EdgeKind = "verified_by"
	EdgeDeclaredIn   EdgeKind = "declared_in"
	EdgeChangedBy    EdgeKind = "changed_by"
	EdgeIntroducedBy EdgeKind = "introduced_by"
	EdgeCalls        EdgeKind = "calls"
	EdgeImports      EdgeKind = "imports"
	EdgeUses         EdgeKind = "uses"
)

// Authority records how a node or edge was obtained. Canonical relations come
// from certified artifacts; derived ones from deterministic analysis (e.g. Go
// AST); candidate ones from heuristics or LLM inference. A candidate relation
// can never silently become canonical.
type Authority string

const (
	AuthorityCanonical Authority = "canonical"
	AuthorityDerived   Authority = "derived"
	AuthorityCandidate Authority = "candidate"
)

// NodeID is a stable textual node identifier: "REQ-042", "CHG-104",
// "sym:internal/auth.Service.Login", "file:internal/auth/login.go",
// "pkg:internal/auth", "test:internal/auth.TestLoginLockout",
// "ev:EVD-ab12cd34ef56", "scn:REQ-042/1".
type NodeID string

// Node is one graph node. Body holds the canonical text of the artifact
// section for spec-side nodes ("" for code nodes); Attrs carries
// kind-specific metadata (class, status, path, package, signature, severity).
type Node struct {
	ID        NodeID
	Kind      NodeKind
	Title     string
	Body      string
	Attrs     map[string]string
	Authority Authority
	Origin    string
	ChangeID  string
}

// Edge is one directed relation.
type Edge struct {
	From, To  NodeID
	Kind      EdgeKind
	Authority Authority
	Origin    string
	ChangeID  string
	Attrs     map[string]string
}

// Direction selects traversal direction.
type Direction int

const (
	Out Direction = iota
	In
	Both
)

// TraverseOpt bounds a Neighbors traversal. Zero values mean: depth 1, Both,
// the semantic edge set, all node kinds, 200 nodes, any authority.
type TraverseOpt struct {
	MaxDepth     int
	Direction    Direction
	EdgeKinds    []EdgeKind
	NodeKinds    []NodeKind
	MaxNodes     int
	MinAuthority Authority
}

// Subgraph is a traversal result. Depth maps each node to its BFS depth from
// the center; Via maps it to its BFS parent (the "why is this here" path).
type Subgraph struct {
	Nodes     []Node
	Edges     []Edge
	Depth     map[NodeID]int
	Via       map[NodeID]NodeID
	Truncated bool
}

// RootInfo binds an index to the tree it was built from. Queries must refuse
// to present a stale cache as current.
type RootInfo struct {
	IndexedCommit   string
	TreeFingerprint string
	SchemaVersion   int
	BuiltAt         time.Time
	Stale           bool
}

// NodeFilter selects nodes by kind, attribute equality, and originating
// change. Empty fields match everything.
type NodeFilter struct {
	Kinds    []NodeKind
	Attrs    map[string]string
	ChangeID string
}

// SearchOpt bounds a full-text search.
type SearchOpt struct {
	Kinds []NodeKind
	Limit int
}

// Hit is one full-text search result.
type Hit struct {
	ID        NodeID
	Kind      NodeKind
	Title     string
	Score     float64
	Snippet   string
	Origin    string
	Authority Authority
}

// EvidenceRow summarizes one evidence record for a requirement, with
// freshness computed at query time against the current tree (never stored).
type EvidenceRow struct {
	ID           string
	Kind         string
	Result       string
	Fresh        bool
	Reusable     bool
	ChangeID     string
	CreatedAt    string
	Requirements []NodeID
}

// FindingFilter selects findings.
type FindingFilter struct {
	ChangeID string
	Status   string
	Blocking bool
	Critic   string
}

// FindingRow is one finding as served by the index.
type FindingRow struct {
	ID                string
	ChangeID          string
	Critic            string
	ProposedSeverity  string
	Confidence        float64
	EffectiveSeverity string
	Blocking          bool
	Status            string
	Resolution        string
	SubjectID         NodeID
	Rationale         string
}

// IndexStats reports node/edge counts by kind and the critic health metric
// (false-positive rate = resolved not_an_issue / resolved total).
type IndexStats struct {
	Nodes        map[NodeKind]int
	Edges        map[EdgeKind]int
	CriticFPRate map[string]float64
}

// Querier is the shared read API over the semantic graph. Implementations
// are read-only and root-bound; Close releases the underlying store.
type Querier interface {
	Root() RootInfo
	Node(id NodeID) (Node, bool, error)
	Nodes(f NodeFilter) ([]Node, error)
	Neighbors(center NodeID, opt TraverseOpt) (Subgraph, error)
	Search(query string, opt SearchOpt) ([]Hit, error)
	EvidenceFor(req NodeID) ([]EvidenceRow, error)
	Findings(f FindingFilter) ([]FindingRow, error)
	ResolveSymbol(name string) ([]Node, error)
	Stats() (IndexStats, error)
	Close() error
}
