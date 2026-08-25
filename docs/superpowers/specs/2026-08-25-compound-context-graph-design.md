# Compound Context Graph Design

**Date:** 2026-08-25
**Status:** Approved in conversation

## Objective

Make the Graph view express DDD boundaries directly. A bounded context is a
container, not a peer node connected to its contents by ordinary edges. The
view remains a single detailed graph; there is no separate strategic mode.

## Visual hierarchy

The graph uses Cytoscape compound nodes with exactly two container levels:

```text
Context
├── context-owned domain nodes
└── Capability
    ├── notions
    ├── intents
    ├── scenarios
    └── constraints
```

- Contexts are root compound nodes.
- Capabilities are compound children of their owning context.
- A context-owned notion, intent, scenario or constraint is a direct child of
  the context.
- A capability-owned notion, intent or constraint is a direct child of the
  capability.
- A scenario has the same context or capability parent as its owning intent;
  intents do not become a third container level.
- Project constraints, production code and tests stay outside every context.
- Context and capability containers remain selectable and link to the
  Contexts page.

## Authoritative parent contract

`GraphNodeView` gains a required `parent: GraphKey | null` field. Rust derives
it from the validated Telos model when building the view snapshot; it is never
persisted in `.tel` files and never inferred by an LLM.

The snapshot builder must guarantee:

1. A context has no parent.
2. A capability has exactly its declared context as parent.
3. Every context- or capability-owned domain node has exactly that owner as
   parent.
4. Every scenario has its intent owner's context or capability as parent.
5. Project constraints, code and tests have no parent.
6. Every non-null parent names an existing context or capability node.
7. The hierarchy is acyclic and at most two containers deep.

The TypeScript mirror and live payload validator treat `parent` as required,
so a stale or malformed hierarchy fails closed before replacing the visible
snapshot.

## Visible relations

`belongs-to` remains part of the core semantic graph for checks, queries and
impact analysis, but the Graph view does not render or offer it as a relation
filter. Compound containment is its sole visual representation.

All other relations retain their canonical direction. `depends-on` may join
context containers; `maps-to` may cross context boundaries between notions;
`implements` and `proves` may cross from an external code or test node into a
context.

## Expansion and collapse

All contexts and capabilities start expanded in a new browser session. Each
container header has an expand/collapse control, and the toolbar adds `Expand
all` and `Collapse all` actions.

Collapse state is stored in `sessionStorage` as container graph keys. On live
reload, keys that still exist are retained and unknown keys are discarded. A
collapsed context hides all descendants while retaining the remembered state
of its capabilities for the next expansion.

The frontend builds a pure visible-graph projection from the immutable
snapshot and the collapsed-key set:

1. Hide descendants of collapsed containers.
2. Redirect each hidden edge endpoint to its nearest visible ancestor.
3. Drop edges whose redirected endpoints are identical.
4. Group remaining edges by visible source, relation and visible target.
5. Preserve every original edge as a member of the resulting visible edge.

An aggregated edge displays its relation and a count when the count is greater
than one. Its selection panel lists the original source, relation and target
for every member, preserving exact traceability.

## Filtering, selection and counts

Relation filtering operates on the visible projected edges. Context and
capability containers are never removed by a filter; unrelated elements are
dimmed as today. `belongs-to` is absent from the filter options.

The page summary reports visible nodes and visible aggregated relations. When
an edge is selected, the panel also reports its number of underlying semantic
relations. Rebuilding the projection clears a selection only when its visible
node or edge no longer exists.

## Layout and styling

Use Cytoscape's native compound-node support through the `parent` data field.
Replace Dagre with the built-in compound-aware CoSE layout. Before the first
CoSE run, assign stable initial positions from graph keys sorted by kind and
id, then run CoSE with `randomize: false`. Expansion, collapse and explicit
`Re-layout` rerun the same deterministic setup.

Contexts use a strong labelled boundary and capabilities use a lighter nested
boundary. Ordinary node shapes and colors continue to express notion, intent,
scenario, constraint, code and test kinds.

## Implementation boundaries

- Rust owns semantic parent derivation and snapshot correctness.
- A pure TypeScript projection owns visibility, endpoint redirection and edge
  aggregation.
- Cytoscape conversion owns only renderer-specific element data.
- The Vue component owns controls, session state and selection events.
- No collapse plugin, fake overlay boxes or second graph mode is introduced.

## Verification

Rust tests must prove every parent rule against the multi-context fixtures,
including context-owned notions, capability-owned nodes, project constraints,
code, tests and scenarios.

Frontend tests must prove:

- two-level compound element generation;
- rejection of missing, invalid or non-container parents;
- complete and per-container expand/collapse behavior;
- endpoint redirection to the nearest visible ancestor;
- aggregation without loss of original relation details;
- removal of redirected self-loops;
- omission of `belongs-to` from rendering, filtering and visible counts;
- preservation and pruning of session collapse keys;
- live payload acceptance for the real context/capability shape.

Completion requires frontend unit tests, typecheck and production build, Rust
view tests, and a live Tamagotchi preview showing both contexts, nested
capabilities, cross-context mappings and external code/test nodes.

## Out of scope

- A strategic/detail mode switch.
- Editing the model from the graph.
- Persisting expansion state beyond the browser session.
- Arbitrary third-level compound containers.
- Changing the core `belongs-to` relation or DDD ownership rules.
