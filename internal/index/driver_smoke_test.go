package index

import (
	"database/sql"
	"testing"

	_ "modernc.org/sqlite"
)

// TestDriverSmoke pins the two properties the knowledge layer depends on:
// the pure-Go driver opens without CGO, and FTS5 is compiled in. It runs
// before any feature code exists so a driver regression fails loudly.
func TestDriverSmoke(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	defer db.Close()

	if _, err := db.Exec(`CREATE VIRTUAL TABLE fts USING fts5(id UNINDEXED, title, body)`); err != nil {
		t.Fatalf("FTS5 unavailable in this build: %v", err)
	}
	if _, err := db.Exec(`INSERT INTO fts(id, title, body) VALUES ('REQ-042', 'Locked accounts', 'Locked accounts cannot authenticate.')`); err != nil {
		t.Fatalf("insert: %v", err)
	}
	var id string
	if err := db.QueryRow(`SELECT id FROM fts WHERE fts MATCH 'authenticate'`).Scan(&id); err != nil {
		t.Fatalf("match query: %v", err)
	}
	if id != "REQ-042" {
		t.Fatalf("match returned %q, want REQ-042", id)
	}
}
