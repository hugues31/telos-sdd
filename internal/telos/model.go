package telos

import "time"

const (
	managedStart = "<!-- telos:managed:start -->"
	managedEnd   = "<!-- telos:managed:end -->"
)

type Config struct {
	Agents               []string
	VerificationCommands []string
}

type Lock struct {
	RootHash  string       `json:"root_hash"`
	Artifacts []LockedFile `json:"artifacts"`
}

type LockedFile struct {
	ID      string   `json:"id"`
	Kind    string   `json:"kind"`
	Path    string   `json:"path"`
	Hash    string   `json:"hash"`
	Parents []string `json:"parents,omitempty"`
}

type Event struct {
	ID       string         `json:"id"`
	At       time.Time      `json:"at"`
	Type     string         `json:"type"`
	Subject  string         `json:"subject,omitempty"`
	Data     map[string]any `json:"data,omitempty"`
	RootHash string         `json:"root_hash,omitempty"`
}

type State struct {
	RootHash    string            `json:"root_hash"`
	Events      int               `json:"events"`
	LatestEvent string            `json:"latest_event,omitempty"`
	Status      map[string]string `json:"status"`
}

type ArtifactMeta struct {
	ID         string
	Kind       string
	Status     string
	Revision   int
	Intent     string
	Flow       string
	Supersedes string
	Parents    []string
}

type TestPlan struct {
	Spec      string     `json:"spec"`
	Feature   string     `json:"feature"`
	Scenarios []Scenario `json:"scenarios"`
	Coverage  []Coverage `json:"coverage,omitempty"`
}

type Scenario struct {
	ID    string   `json:"id"`
	Rule  string   `json:"rule"`
	Name  string   `json:"name"`
	Tags  []string `json:"tags,omitempty"`
	Given []string `json:"given"`
	When  []string `json:"when"`
	Then  []string `json:"then"`
}

type Coverage struct {
	Rule      string `json:"rule"`
	Category  string `json:"category"`
	Status    string `json:"status"`
	Rationale string `json:"rationale,omitempty"`
}

type Flow struct {
	ID             string            `json:"id"`
	Status         string            `json:"status"`
	Phase          string            `json:"phase"`
	Request        string            `json:"request"`
	Brainstorm     string            `json:"brainstorm,omitempty"`
	Intent         string            `json:"intent,omitempty"`
	Specs          []string          `json:"specs,omitempty"`
	Change         string            `json:"change,omitempty"`
	IntentReview   string            `json:"intent_review,omitempty"`
	ContractReview string            `json:"contract_review,omitempty"`
	DraftHashes    map[string]string `json:"draft_hashes,omitempty"`
	Verdict        string            `json:"verdict,omitempty"`
	Created        string            `json:"created"`
	Updated        string            `json:"updated"`
}

type Change struct {
	ID                string   `json:"id"`
	Flow              string   `json:"flow,omitempty"`
	Intent            string   `json:"intent"`
	Specs             []string `json:"specs"`
	Base              string   `json:"base"`
	Status            string   `json:"status"`
	Started           string   `json:"started"`
	Completed         string   `json:"completed,omitempty"`
	SourceBaseRoot    string   `json:"source_base_root,omitempty"`
	SourceCurrentRoot string   `json:"source_current_root,omitempty"`
	ContextHash       string   `json:"context_hash,omitempty"`
	Transactions      []string `json:"transactions,omitempty"`
}

type RepositoryLock struct {
	RootHash string           `json:"root_hash"`
	Files    []RepositoryFile `json:"files"`
}

type RepositoryFile struct {
	Path string `json:"path"`
	Hash string `json:"hash"`
	Mode uint32 `json:"mode"`
}

type Mutation struct {
	ID         string   `json:"id"`
	Change     string   `json:"change"`
	PatchHash  string   `json:"patch_hash"`
	PatchPath  string   `json:"patch_path"`
	BeforeRoot string   `json:"before_root"`
	AfterRoot  string   `json:"after_root"`
	Paths      []string `json:"paths"`
	Rules      []string `json:"rules"`
	Scenarios  []string `json:"scenarios"`
	At         string   `json:"at"`
}

type InstallManifest struct {
	Files map[string]string `json:"files"`
}
