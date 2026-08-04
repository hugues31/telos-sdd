package telos

const (
	managedStart = "<!-- telos:managed:start -->"
	managedEnd   = "<!-- telos:managed:end -->"
)

const (
	configFile  = "telos.toml"
	stateFile   = ".telos/state.json"
	specDir     = "spec"
	productFile = "spec/PRODUCT.md"
)

type Config struct {
	Agents       []string
	TestCommands []string
	TestFiles    []string
	Untraced     []string
}

// Snapshot records a declared tree state: a root hash over the sorted
// path+NUL+hash+LF records plus the per-file hashes used for precise
// error reporting.
type Snapshot struct {
	Root  string            `json:"root"`
	Files map[string]string `json:"files"`
}

// State is the only content of .telos/. It is committed to Git so merges,
// pulls, and CI checkouts stay coherent with the tree they travel with.
// Like the approved spec root and the declared code root, Green and Red record
// facts that are decisions or witnessed observations, never derivable content:
// Green is the last code root at which the broker saw the test suite pass, and
// Red holds, per rule, the exact test bytes the broker saw fail.
type State struct {
	Version int                    `json:"version"`
	Spec    Snapshot               `json:"spec"`
	Code    Snapshot               `json:"code"`
	Review  string                 `json:"review,omitempty"`
	Green   string                 `json:"green,omitempty"`
	Red     map[string]RedEvidence `json:"red,omitempty"`
}

// RedEvidence seals a rule's failing test: the hash of every test file that
// referenced the rule when the broker witnessed the suite fail. Until the rule
// is proven, no patch may touch these files except a test-only patch that
// fails again — only the implementation is allowed to turn red into green.
type RedEvidence struct {
	Tests map[string]string `json:"tests"`
}
