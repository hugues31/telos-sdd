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
type State struct {
	Version int      `json:"version"`
	Spec    Snapshot `json:"spec"`
	Code    Snapshot `json:"code"`
	Review  string   `json:"review,omitempty"`
}
