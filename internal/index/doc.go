// Package index implements graph.Querier over a derived SQLite database at
// .telos/cache/index.db. The database is disposable by definition: deleting
// it and running `telos index rebuild` must restore the complete graph from
// certified artifacts alone. It is root-bound: queries refuse to present a
// stale cache as current. The implementation arrives at M5; this package
// currently pins the driver choice (pure-Go modernc.org/sqlite, FTS5
// verified by the smoke test).
package index
