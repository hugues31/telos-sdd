package telos

import "time"

const (
	configVersion = 1
	managedStart  = "<!-- telos:managed:start -->"
	managedEnd    = "<!-- telos:managed:end -->"
)

type Config struct {
	Version              int
	Profile              string
	Agents               []string
	VerificationCommands []string
}

type Lock struct {
	Version   int          `json:"version"`
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
	Version  int            `json:"version"`
	ID       string         `json:"id"`
	At       time.Time      `json:"at"`
	Type     string         `json:"type"`
	Subject  string         `json:"subject,omitempty"`
	Data     map[string]any `json:"data,omitempty"`
	RootHash string         `json:"root_hash,omitempty"`
}

type State struct {
	Version     int               `json:"version"`
	RootHash    string            `json:"root_hash"`
	Events      int               `json:"events"`
	LatestEvent string            `json:"latest_event,omitempty"`
	Status      map[string]string `json:"status"`
}

type ArtifactMeta struct {
	ID       string
	Kind     string
	Status   string
	Revision int
	Intent   string
	Parents  []string
}

type TestPlan struct {
	Version   int        `json:"version"`
	Spec      string     `json:"spec"`
	Feature   string     `json:"feature"`
	Scenarios []Scenario `json:"scenarios"`
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

type Change struct {
	Version int      `json:"version"`
	ID      string   `json:"id"`
	Intent  string   `json:"intent"`
	Specs   []string `json:"specs"`
	Base    string   `json:"base"`
	Status  string   `json:"status"`
	Started string   `json:"started"`
}

type InstallManifest struct {
	Version int               `json:"version"`
	Files   map[string]string `json:"files"`
}
