# Compound Context Graph Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render bounded contexts and capabilities as collapsible compound containers in the single Telos Graph view, with deterministic ownership and lossless aggregated relations.

**Architecture:** Rust adds an authoritative optional parent to every graph node. A pure TypeScript projection turns the immutable semantic graph plus collapse state into visible nodes and aggregated edges, while Cytoscape only renders the resulting compound graph. Vue owns interaction and session persistence.

**Tech Stack:** Rust, serde, Vue 3, TypeScript, Vitest, Cytoscape 3 built-in compound nodes and CoSE layout.

**Spec:** `docs/superpowers/specs/2026-08-25-compound-context-graph-design.md`

## Global Constraints

- Keep one Graph view; do not add strategic/detail modes.
- Contexts contain capabilities; both are compound containers.
- Project constraints, code and tests stay outside contexts.
- Keep `belongs-to` in the semantic snapshot but never render or filter it.
- Use no collapse plugin, fake overlay box, or additional layout dependency.
- All hierarchy, redirection and aggregation rules must be deterministic and testable without a browser.
- Preserve unrelated user changes in the already-dirty worktree; use scoped diffs instead of intermediate code commits.

---

### Task 1: Authoritative graph parents in the Rust snapshot

**Files:**
- Modify: `crates/telos/src/view/model.rs`
- Modify: `crates/telos/src/view/data.rs`
- Modify: `crates/telos/tests/view_export.rs`
- Modify: `frontend/src/data/types.ts`
- Modify: `frontend/src/data/live.ts`
- Modify: `frontend/src/data/live.test.ts`
- Modify: `frontend/public/data.js`

**Interfaces:**
- Produces: `GraphNodeView { key: GraphKey, label: String, parent: Option<GraphKey> }` in Rust.
- Produces: `GraphNodeView { key: GraphKey; label: string; parent: GraphKey | null }` in TypeScript.
- Produces: `fn graph_parent(model: &TelosModel, node: &NodeRef) -> Option<GraphKey>`.

- [ ] **Step 1: Write failing Rust parent-contract tests**

Add literal assertions in `view/model.rs` proving these mappings from the Billing fixture:

```rust
assert_eq!(parent(&snapshot, GraphKey::Context("billing".into())), None);
assert_eq!(
    parent(&snapshot, GraphKey::Capability("billing/invoicing".into())),
    Some(GraphKey::Context("billing".into())),
);
assert_eq!(
    parent(&snapshot, GraphKey::Intent("INT-0017".into())),
    Some(GraphKey::Capability("billing/invoicing".into())),
);
assert_eq!(
    parent(&snapshot, GraphKey::Scenario("SCN-0091".into())),
    Some(GraphKey::Capability("billing/invoicing".into())),
);
assert_eq!(
    parent(&snapshot, GraphKey::Constraint("CON-0003".into())),
    Some(GraphKey::Context("billing".into())),
);
assert_eq!(parent(&snapshot, GraphKey::Code("src/billing/invoice.rs".into())), None);
```

- [ ] **Step 2: Run the Rust test and observe the missing field failure**

Run: `rtk cargo test -p telos view::model::tests::snapshot_assigns_authoritative_compound_parents`

Expected: compilation fails because `GraphNodeView.parent` does not exist.

- [ ] **Step 3: Implement parent derivation in Rust**

Use the validated owner maps, not graph-edge scanning:

```rust
fn owner_key(owner: &Owner) -> GraphKey {
    match owner.capability_ref() {
        Some(capability) => GraphKey::Capability(capability.to_string()),
        None => GraphKey::Context(owner.context.to_string()),
    }
}

fn graph_parent(model: &TelosModel, node: &NodeRef) -> Option<GraphKey> {
    match node {
        NodeRef::Capability(id) => Some(GraphKey::Context(id.context.to_string())),
        NodeRef::QualifiedNotion(id) => model.notion_owners.get(id).map(owner_key),
        NodeRef::Intent(id) => model.intent_owners.get(id).map(owner_key),
        NodeRef::Scenario(id) => model
            .scenario_owner
            .get(id)
            .and_then(|intent| model.intent_owners.get(intent))
            .map(owner_key),
        NodeRef::Constraint(id) => model
            .constraint_owners
            .get(id)
            .and_then(Option::as_ref)
            .map(owner_key),
        NodeRef::Context(_) | NodeRef::Notion(_) | NodeRef::Code(_) | NodeRef::Test(_) => None,
    }
}
```

Set `parent: graph_parent(model, node)` in `graph_node`.

- [ ] **Step 4: Run Rust view tests**

Run: `rtk cargo test -p telos view::model::tests`

Expected: all view-model tests pass.

- [ ] **Step 5: Write failing frontend payload tests**

Update valid fixtures with literal parents, then add malformed cases for a missing parent field and for a parent whose kind is not `context` or `capability`.

- [ ] **Step 6: Update TypeScript types and runtime validation**

Require `parent`, validate `null` or `isGraphKey`, restrict non-null parent kinds to containers, and validate after all nodes are known that every parent exists.

- [ ] **Step 7: Run contract tests**

Run: `rtk npm test -- --run src/data/live.test.ts`

Expected: all live payload tests pass.

- [ ] **Step 8: Update static payload fixtures and scoped diff check**

Add `parent` to `frontend/public/data.js` and Rust export fixtures, then run:

```bash
rtk git diff --check -- crates/telos/src/view frontend/src/data frontend/public/data.js crates/telos/tests/view_export.rs
```

---

### Task 2: Pure visible-graph projection

**Files:**
- Create: `frontend/src/graph/projection.ts`
- Create: `frontend/src/graph/projection.test.ts`

**Interfaces:**
- Consumes: authoritative `GraphNodeView.parent` and semantic `GraphEdgeView[]`.
- Produces: `VisibleGraphEdge { from, relation, to, members }`.
- Produces: `VisibleGraph { nodes, edges }`.
- Produces: `projectVisibleGraph(nodes, edges, collapsedIds)`.

- [ ] **Step 1: Write failing projection tests with two contexts**

Use a hand-built fixture containing two contexts, nested capabilities, domain nodes, one external test, `belongs-to`, cross-context `maps-to`, and duplicate redirected `uses` edges. Assert literal visible node ids, redirected endpoints, member arrays and counts for expanded, capability-collapsed and context-collapsed states.

- [ ] **Step 2: Run the projection test and observe the missing module failure**

Run: `rtk npm test -- --run src/graph/projection.test.ts`

Expected: failure because `projection.ts` does not exist.

- [ ] **Step 3: Implement the pure projection**

Define:

```ts
export type VisibleGraphRelation = Exclude<GraphRelation, 'belongs-to'>;

export interface VisibleGraphEdge {
  from: GraphKey;
  relation: VisibleGraphRelation;
  to: GraphKey;
  members: GraphEdgeView[];
}

export interface VisibleGraph {
  nodes: GraphNodeView[];
  edges: VisibleGraphEdge[];
}

export function projectVisibleGraph(
  nodes: GraphNodeView[],
  edges: GraphEdgeView[],
  collapsedIds: ReadonlySet<string>,
): VisibleGraph;
```

Build a node-id map, determine the nearest visible ancestor for every node,
drop `belongs-to`, redirect endpoints, drop self-loops, group by
`source/relation/target`, preserve member order, and sort output by stable ids.

- [ ] **Step 4: Run projection tests**

Run: `rtk npm test -- --run src/graph/projection.test.ts`

Expected: all projection tests pass.

- [ ] **Step 5: Add invalid-hierarchy tests**

Assert that projection throws a precise error for a missing parent, a non-container parent, a parent cycle, and a hierarchy deeper than context/capability/content.

- [ ] **Step 6: Run projection tests again**

Run: `rtk npm test -- --run src/graph/projection.test.ts`

Expected: all projection tests pass.

---

### Task 3: Compound Cytoscape elements, filtering and layout

**Files:**
- Modify: `frontend/src/graph/elements.ts`
- Modify: `frontend/src/graph/elements.test.ts`
- Modify: `frontend/src/graph/relations.ts`
- Modify: `frontend/src/graph/relations.test.ts`
- Modify: `frontend/src/graph/layout.ts`
- Modify: `frontend/src/graph/stylesheet.ts`

**Interfaces:**
- Consumes: `VisibleGraphEdge[]` rather than raw semantic edges.
- Produces: Cytoscape node data with `parent` for compound children.
- Produces: edge data with `count` and `members`.
- Produces: `COSE_LAYOUT_OPTIONS` and `runCompoundLayout(cy)`.

- [ ] **Step 1: Update element tests first**

Assert that container nodes are emitted before children, child data contains the canonical parent node id, external nodes omit parent, and aggregated edge data contains a literal count and original members.

- [ ] **Step 2: Run element and relation tests to observe failures**

Run: `rtk npm test -- --run src/graph/elements.test.ts src/graph/relations.test.ts`

Expected: failures because current elements are flat and `belongs-to` remains a filter option.

- [ ] **Step 3: Implement compound element conversion**

Change `buildGraphElements` to accept projected edges, topologically emit contexts, then capabilities, then ordinary nodes, and map `parent` through `nodeId`. Keep stable visible-edge ids based on redirected endpoints and relation.

- [ ] **Step 4: Remove `belongs-to` from relation options and visible filtering**

Keep `GRAPH_RELATIONS` unchanged in the payload contract. Add a display-only constant excluding `belongs-to` and use it in `relationOptionsFor`.

- [ ] **Step 5: Replace Dagre with deterministic compound CoSE**

Register no extension. Seed sorted graph nodes on a stable grid, then run built-in CoSE with:

```ts
export const COSE_LAYOUT_OPTIONS = {
  name: 'cose',
  randomize: false,
  animate: false,
  fit: true,
  padding: 32,
  componentSpacing: 80,
  nestingFactor: 1.2,
} as const;
```

- [ ] **Step 6: Add compound styles**

Style context and capability parents with labelled top-aligned boundaries, padding and different border strengths. Restrict fixed 44px dimensions and bottom labels to non-parent nodes. Display aggregated labels as `relation ×N` only when `count > 1`.

- [ ] **Step 7: Run graph unit tests**

Run: `rtk npm test -- --run src/graph`

Expected: all graph unit tests pass.

---

### Task 4: Collapse controls, session persistence and selection details

**Files:**
- Create: `frontend/src/graph/collapse.ts`
- Create: `frontend/src/graph/collapse.test.ts`
- Modify: `frontend/src/graph/CytoGraph.vue`
- Modify: `frontend/src/graph/selection.ts`
- Modify: `frontend/src/graph/selection.test.ts`
- Modify: `frontend/src/pages/GraphPage.vue`

**Interfaces:**
- Produces: `normalizeCollapsedIds(nodes, storedIds)`.
- Produces: `containerNodeIds(nodes)`.
- `CytoGraph` consumes `collapsedIds: string[]` to render container state.
- `CytoGraph` emits `toggle-container`, `expand-all`, `collapse-all`.
- Edge selection includes `members: GraphEdgeView[]`.

- [ ] **Step 1: Write failing collapse-state tests**

Assert that only existing context/capability ids survive normalization, ordinary node ids are rejected, `collapse all` returns every container id, and expanding a context does not erase remembered collapsed capability ids.

- [ ] **Step 2: Run collapse tests and observe the missing module failure**

Run: `rtk npm test -- --run src/graph/collapse.test.ts`

Expected: failure because `collapse.ts` does not exist.

- [ ] **Step 3: Implement collapse helpers and session storage adapter**

Use storage key `telos.graph.collapsed.v1`, store a sorted JSON string array, catch unavailable or malformed storage, and fall back to an empty set. Prune stored ids whenever live nodes change.

- [ ] **Step 4: Wire the visible projection into GraphPage**

Compute `visibleGraph` from raw snapshot nodes/edges and collapsed ids. Base summary, relation options, filters and selection validity on visible data. Keep all containers visible under relation filtering.

- [ ] **Step 5: Add controls to CytoGraph**

Add `Expand all` and `Collapse all` toolbar buttons. Render a `+` or `−` prefix in each compound label and detect a tap in the compound header band to emit `toggle-container`; ordinary taps retain selection behavior. Rerun compound layout after every projection change.

- [ ] **Step 6: Render aggregated edge details**

Show the aggregate count in the selection panel and list every member using existing `EntityLink` components for source and target. A single-member edge keeps the compact current presentation.

- [ ] **Step 7: Update selection tests**

Prove selection survives a live refresh when its visible id remains, and clears when collapse removes the selected node or changes the selected aggregate edge id.

- [ ] **Step 8: Run focused page and graph tests**

Run: `rtk npm test -- --run src/graph src/pages/page-data.test.ts`

Expected: all focused tests pass.

---

### Task 5: Full verification and Tamagotchi preview

**Files:**
- Modify only if verification exposes a contract defect in the files above.

**Interfaces:**
- Consumes: completed Rust snapshot and frontend compound renderer.
- Produces: rebuilt embedded frontend and a live server at `http://127.0.0.1:3000/`.

- [ ] **Step 1: Run complete frontend verification**

```bash
rtk npm test -- --run
rtk npm run typecheck
rtk npm run build
```

Expected: all tests pass, typecheck exits zero, Vite builds `dist/assets/app.js`.

- [ ] **Step 2: Run Rust formatting and view tests**

```bash
rtk cargo fmt --all --check
rtk cargo test -p telos view::
rtk cargo test -p telos --test view_export --test view_server
rtk cargo clippy -p telos --all-targets -- -D warnings
```

Expected: every command exits zero.

- [ ] **Step 3: Rebuild the embedded frontend binary**

Run: `rtk cargo build -p telos`

Expected: the `telos` binary embeds the freshly built compound graph bundle.

- [ ] **Step 4: Check Tamagotchi model integrity**

From `/home/hugues/Bureau/telos-tamagotchi`, run:

```bash
rtk env PATH=/home/hugues/Bureau/telos-tamagotchi/.venv/bin:/usr/bin:/bin /home/hugues/Bureau/telos-sdd/target/debug/telos check --sealed --json
```

Expected: success with a coherent sealed model.

- [ ] **Step 5: Start the preview and verify served assets**

From `/home/hugues/Bureau/telos-tamagotchi`, start:

```bash
rtk env PATH=/home/hugues/Bureau/telos-tamagotchi/.venv/bin:/usr/bin:/bin /home/hugues/Bureau/telos-sdd/target/debug/telos view --port 3000 --json
```

Verify HTTP 200, no live reload errors, and that served `data.js` contains non-null capability and domain-node parents.

- [ ] **Step 6: Open the browser preview for user review**

Run: `rtk xdg-open http://127.0.0.1:3000/`

Leave the server running so the user can inspect nested contexts, collapse controls, relation aggregation and external code/test nodes.
