package index

import (
	"database/sql"
	"encoding/json"
	"sort"
	"strings"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/evidence"
	"github.com/hugues31/telos-sdd/internal/graph"
)

// semanticEdges is the default traversal set: everything except the code
// analysis kinds.
var semanticEdges = []graph.EdgeKind{
	graph.EdgeMotivates, graph.EdgeRefines, graph.EdgeSupersedes, graph.EdgeDependsOn,
	graph.EdgeConstrains, graph.EdgeConflicts, graph.EdgeImplements, graph.EdgeVerifiedBy,
	graph.EdgeChangedBy, graph.EdgeIntroducedBy,
}

func scanNode(row interface{ Scan(...any) error }) (graph.Node, error) {
	var n graph.Node
	var attrs string
	err := row.Scan((*string)(&n.ID), (*string)(&n.Kind), &n.Title, &n.Body, &attrs, (*string)(&n.Authority), &n.Origin, &n.ChangeID)
	if err != nil {
		return n, err
	}
	_ = json.Unmarshal([]byte(attrs), &n.Attrs)
	return n, nil
}

const nodeColumns = `id,kind,title,body,attrs,authority,origin,change_id`

// Node fetches one node by id.
func (d *DB) Node(id graph.NodeID) (graph.Node, bool, error) {
	row := d.sql.QueryRow(`SELECT `+nodeColumns+` FROM nodes WHERE id=?`, string(id))
	n, err := scanNode(row)
	if err == sql.ErrNoRows {
		return n, false, nil
	}
	return n, err == nil, err
}

// Nodes lists nodes matching the filter.
func (d *DB) Nodes(f graph.NodeFilter) ([]graph.Node, error) {
	query := `SELECT ` + nodeColumns + ` FROM nodes`
	var conds []string
	var args []any
	if len(f.Kinds) > 0 {
		marks := make([]string, len(f.Kinds))
		for i, k := range f.Kinds {
			marks[i] = "?"
			args = append(args, string(k))
		}
		conds = append(conds, "kind IN ("+strings.Join(marks, ",")+")")
	}
	if f.ChangeID != "" {
		conds = append(conds, "change_id=?")
		args = append(args, f.ChangeID)
	}
	if len(conds) > 0 {
		query += " WHERE " + strings.Join(conds, " AND ")
	}
	query += " ORDER BY id"
	rows, err := d.sql.Query(query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []graph.Node
	for rows.Next() {
		n, err := scanNode(rows)
		if err != nil {
			return nil, err
		}
		match := true
		for k, v := range f.Attrs {
			if n.Attrs[k] != v {
				match = false
			}
		}
		if match {
			out = append(out, n)
		}
	}
	return out, rows.Err()
}

// Neighbors runs a bounded BFS from center.
func (d *DB) Neighbors(center graph.NodeID, opt graph.TraverseOpt) (graph.Subgraph, error) {
	sub := graph.Subgraph{Depth: map[graph.NodeID]int{}, Via: map[graph.NodeID]graph.NodeID{}}
	if _, ok, err := d.Node(center); err != nil {
		return sub, err
	} else if !ok {
		return sub, coded.New("TELOS_NODE_NOT_FOUND", "no node "+string(center)+"; use `telos search` to locate it")
	}
	maxDepth := opt.MaxDepth
	if maxDepth <= 0 {
		maxDepth = 1
	}
	if maxDepth > 4 {
		maxDepth = 4
	}
	maxNodes := opt.MaxNodes
	if maxNodes <= 0 {
		maxNodes = 200
	}
	kinds := opt.EdgeKinds
	if len(kinds) == 0 {
		kinds = semanticEdges
	}
	kindSet := map[graph.EdgeKind]bool{}
	for _, k := range kinds {
		kindSet[k] = true
	}

	sub.Depth[center] = 0
	queue := []graph.NodeID{center}
	seenEdge := map[[3]string]bool{}
	for len(queue) > 0 && len(sub.Depth) <= maxNodes {
		current := queue[0]
		queue = queue[1:]
		depth := sub.Depth[current]
		if depth >= maxDepth {
			continue
		}
		rows, err := d.sql.Query(`SELECT src,dst,kind,authority,origin,change_id FROM edges WHERE src=? OR dst=?`, string(current), string(current))
		if err != nil {
			return sub, err
		}
		var found []graph.Edge
		for rows.Next() {
			var e graph.Edge
			if err := rows.Scan((*string)(&e.From), (*string)(&e.To), (*string)(&e.Kind), (*string)(&e.Authority), &e.Origin, &e.ChangeID); err != nil {
				rows.Close()
				return sub, err
			}
			found = append(found, e)
		}
		rows.Close()
		sort.Slice(found, func(i, j int) bool {
			ki := string(found[i].From) + string(found[i].Kind) + string(found[i].To)
			kj := string(found[j].From) + string(found[j].Kind) + string(found[j].To)
			return ki < kj
		})
		for _, e := range found {
			if !kindSet[e.Kind] {
				continue
			}
			if opt.Direction == graph.Out && string(e.From) != string(current) {
				continue
			}
			if opt.Direction == graph.In && string(e.To) != string(current) {
				continue
			}
			other := e.To
			if other == current {
				other = e.From
			}
			key := [3]string{string(e.From), string(e.Kind), string(e.To)}
			if !seenEdge[key] {
				seenEdge[key] = true
				sub.Edges = append(sub.Edges, e)
			}
			if _, visited := sub.Depth[other]; !visited {
				if len(sub.Depth) > maxNodes {
					sub.Truncated = true
					continue
				}
				sub.Depth[other] = depth + 1
				sub.Via[other] = current
				queue = append(queue, other)
			}
		}
	}
	ids := make([]string, 0, len(sub.Depth))
	for id := range sub.Depth {
		ids = append(ids, string(id))
	}
	sort.Strings(ids)
	for _, id := range ids {
		if n, ok, err := d.Node(graph.NodeID(id)); err != nil {
			return sub, err
		} else if ok {
			sub.Nodes = append(sub.Nodes, n)
		}
	}
	return sub, nil
}

// Search runs a full-text query, title weighted over body.
func (d *DB) Search(query string, opt graph.SearchOpt) ([]graph.Hit, error) {
	limit := opt.Limit
	if limit <= 0 {
		limit = 20
	}
	// Escape into a quoted phrase per token to survive FTS syntax characters.
	tokens := strings.Fields(query)
	for i, tok := range tokens {
		tokens[i] = `"` + strings.ReplaceAll(tok, `"`, ``) + `"`
	}
	if len(tokens) == 0 {
		return nil, nil
	}
	match := strings.Join(tokens, " ")
	rows, err := d.sql.Query(`
		SELECT f.id, f.kind, n.title, snippet(fts, 3, '[', ']', '…', 12), bm25(fts, 0, 0, 3.0, 1.0), n.authority, n.origin
		FROM fts f JOIN nodes n ON n.id = f.id
		WHERE fts MATCH ? ORDER BY bm25(fts, 0, 0, 3.0, 1.0) LIMIT ?`, match, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var hits []graph.Hit
	for rows.Next() {
		var h graph.Hit
		var score float64
		if err := rows.Scan((*string)(&h.ID), (*string)(&h.Kind), &h.Title, &h.Snippet, &score, (*string)(&h.Authority), &h.Origin); err != nil {
			return nil, err
		}
		h.Score = -score // bm25 returns negative-is-better
		hits = append(hits, h)
	}
	return hits, rows.Err()
}

// EvidenceFor lists the records citing a requirement, with freshness
// recomputed live against the current HEAD tree (never stored).
func (d *DB) EvidenceFor(req graph.NodeID) ([]graph.EvidenceRow, error) {
	rows, err := d.sql.Query(`
		SELECT e.id, e.kind, e.result, e.reusable, e.change_id, e.created_at, e.record
		FROM evidence e JOIN evidence_reqs r ON r.change_id = e.change_id AND r.key = e.key
		WHERE r.req_id = ? ORDER BY e.created_at, e.id`, string(req))
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	tree, treeErr := d.repo.TreeOf("HEAD")
	var out []graph.EvidenceRow
	for rows.Next() {
		var row graph.EvidenceRow
		var reusable int
		var raw string
		if err := rows.Scan(&row.ID, &row.Kind, &row.Result, &reusable, &row.ChangeID, &row.CreatedAt, &raw); err != nil {
			return nil, err
		}
		row.Reusable = reusable == 1
		row.Requirements = []graph.NodeID{req}
		var record evidence.Record
		if treeErr == nil && json.Unmarshal([]byte(raw), &record) == nil {
			if digest, err := evidence.Recompute(d.repo, tree, d.repo.WorkDir, &record); err == nil {
				row.Fresh = digest == record.DependsOn.ClosureDigest
			}
		}
		out = append(out, row)
	}
	return out, rows.Err()
}

// Findings lists findings matching the filter.
func (d *DB) Findings(f graph.FindingFilter) ([]graph.FindingRow, error) {
	query := `SELECT id,change_id,critic,proposed_severity,confidence,severity,status,resolution,subject,rationale FROM findings`
	var conds []string
	var args []any
	if f.ChangeID != "" {
		conds = append(conds, "change_id=?")
		args = append(args, f.ChangeID)
	}
	if f.Status != "" {
		conds = append(conds, "status=?")
		args = append(args, f.Status)
	}
	if f.Blocking {
		conds = append(conds, "severity='blocking' AND status='open'")
	}
	if f.Critic != "" {
		conds = append(conds, "critic=?")
		args = append(args, f.Critic)
	}
	if len(conds) > 0 {
		query += " WHERE " + strings.Join(conds, " AND ")
	}
	query += " ORDER BY change_id, id"
	rows, err := d.sql.Query(query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []graph.FindingRow
	for rows.Next() {
		var r graph.FindingRow
		var subject string
		if err := rows.Scan(&r.ID, &r.ChangeID, &r.Critic, &r.ProposedSeverity, &r.Confidence, &r.EffectiveSeverity, &r.Status, &r.Resolution, &subject, &r.Rationale); err != nil {
			return nil, err
		}
		r.SubjectID = graph.NodeID(subject)
		r.Blocking = r.EffectiveSeverity == "blocking" && r.Status == "open"
		out = append(out, r)
	}
	return out, rows.Err()
}

// ResolveSymbol finds symbols by exact name, Recv.Name suffix, or node id.
func (d *DB) ResolveSymbol(name string) ([]graph.Node, error) {
	if strings.HasPrefix(name, "sym:") {
		if n, ok, err := d.Node(graph.NodeID(name)); err != nil {
			return nil, err
		} else if ok {
			return []graph.Node{n}, nil
		}
		return nil, nil
	}
	rows, err := d.sql.Query(`SELECT node_id FROM symbols WHERE name=? OR name LIKE ? ORDER BY node_id`, name, "%."+name)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var ids []string
	for rows.Next() {
		var id string
		if err := rows.Scan(&id); err != nil {
			return nil, err
		}
		ids = append(ids, id)
	}
	var out []graph.Node
	for _, id := range ids {
		if n, ok, err := d.Node(graph.NodeID(id)); err != nil {
			return nil, err
		} else if ok {
			out = append(out, n)
		}
	}
	return out, nil
}

// Stats reports counts and the critic false-positive health metric.
func (d *DB) Stats() (graph.IndexStats, error) {
	stats := graph.IndexStats{Nodes: map[graph.NodeKind]int{}, Edges: map[graph.EdgeKind]int{}, CriticFPRate: map[string]float64{}}
	rows, err := d.sql.Query(`SELECT kind, COUNT(*) FROM nodes GROUP BY kind`)
	if err != nil {
		return stats, err
	}
	for rows.Next() {
		var kind string
		var count int
		if err := rows.Scan(&kind, &count); err != nil {
			rows.Close()
			return stats, err
		}
		stats.Nodes[graph.NodeKind(kind)] = count
	}
	rows.Close()
	rows, err = d.sql.Query(`SELECT kind, COUNT(*) FROM edges GROUP BY kind`)
	if err != nil {
		return stats, err
	}
	for rows.Next() {
		var kind string
		var count int
		if err := rows.Scan(&kind, &count); err != nil {
			rows.Close()
			return stats, err
		}
		stats.Edges[graph.EdgeKind(kind)] = count
	}
	rows.Close()
	rows, err = d.sql.Query(`
		SELECT critic,
			SUM(CASE WHEN resolution='not_an_issue' THEN 1 ELSE 0 END),
			SUM(CASE WHEN status='resolved' THEN 1 ELSE 0 END)
		FROM findings WHERE critic != 'human' GROUP BY critic`)
	if err != nil {
		return stats, err
	}
	defer rows.Close()
	for rows.Next() {
		var critic string
		var fp, resolved int
		if err := rows.Scan(&critic, &fp, &resolved); err != nil {
			return stats, err
		}
		if resolved > 0 {
			stats.CriticFPRate[critic] = float64(fp) / float64(resolved)
		}
	}
	return stats, rows.Err()
}

var _ graph.Querier = (*DB)(nil)
