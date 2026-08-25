# Telos View 0.10 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Telos 0.10.0 with a mobile header, keyboard-first global search, localized footer date, graph node finder, canonical intent parity, typed glossary consumers, and a blue/cyan brand palette.

**Architecture:** Rust continues to own `.tel` formatting and adds display-ready statement/scenario fragments to the strict view payload. Vue builds one shared entity search/destination layer used by a global command palette and the graph finder, while focused components own graph focusing, glossary consumer tables, and responsive navigation. Each behavior is introduced test-first before page/component wiring.

**Tech Stack:** Rust 2024, serde, Vue 3 Composition API, TypeScript 5.9, Vue Router 4, Cytoscape 3, Vitest 4, Vite 7, CSS custom properties.

**Spec:** `docs/superpowers/specs/2026-08-25-telos-view-v0.10-design.md`

## Global Constraints

- `.tel` syntax, model semantics, graph relations, routes, and live-reload transport do not change.
- Vue must not parse `.tel` or mirror the complete Rust AST.
- Global search and graph finding must share one index/ranking implementation.
- Graph search selects and centers a node without filtering the graph.
- Existing glossary cards remain; only their consumer presentation changes.
- Blue is the brand color; green remains reserved for healthy/success state.
- New controls must be keyboard accessible, touch friendly, and usable at 48rem and below.
- No new runtime or development dependency is required.
- Every shell command in this repository is prefixed with `rtk`.

## File Structure

- `crates/telos-core/src/emit.rs`: authoritative standalone statement/scenario fragment emission.
- `crates/telos/src/view/model.rs`: Rust view-contract projection of canonical fragments.
- `frontend/src/data/types.ts`, `frontend/src/data/live.ts`: TypeScript contract and strict runtime validation.
- `frontend/src/search/entities.ts`: shared search ranking, keyboard-target guard, and eight-result limit.
- `frontend/src/search/destinations.ts`: one entity-to-route mapping for `EntityLink` and global search.
- `frontend/src/components/GlobalSearch.vue`: global command palette and shortcuts.
- `frontend/src/components/AppHeader.vue`: responsive menu and global-search integration.
- `frontend/src/format/date.ts`: timezone-safe localized build-date formatting.
- `frontend/src/graph/search.ts`: collapsed-ancestor expansion and typed graph-focus query parsing.
- `frontend/src/components/GraphFinder.vue`: graph-local accessible finder.
- `frontend/src/graph/CytoGraph.vue`: post-layout Cytoscape focus operation.
- `frontend/src/pages/IntentDetailPage.vue`: canonical statement/scenario presentation and directional relations.
- `frontend/src/pages/glossary-consumers.ts`: typed consumer row filtering/sorting.
- `frontend/src/components/UsedByTable.vue`: reusable glossary consumer table.
- `frontend/src/styles/tokens.css`: logo-derived blue/cyan light and dark palettes.

---

### Task 1: Canonical Statement and Scenario View Contract

**Files:**
- Modify: `crates/telos-core/src/emit.rs:236-381`
- Modify: `crates/telos/src/view/model.rs:70-105, 213-274, 680-1160`
- Modify: `crates/telos/src/view/data.rs:39-118`
- Modify: `frontend/src/data/types.ts:91-120`
- Modify: `frontend/src/data/live.ts:155-238`
- Modify: `frontend/src/data/live.test.ts:42-93, 560-730`
- Modify: `frontend/public/data.js`
- Test: inline Rust tests in `emit.rs`, `model.rs`, and `data.rs`

**Interfaces:**
- Consumes: `telos_core::model::{Statement, Scenario}` and the existing canonical emitter helpers.
- Produces: `emit_statement_fragment(&Statement) -> String`, public visibility for the existing `emit_scenario_fragment(&Scenario) -> String`, `statement_template(&Statement) -> &'static str`, TypeScript `StatementView`, and `ScenarioView.canonical`.

- [ ] **Step 1: Add failing canonical-fragment emitter tests**

Add tests that call the public functions and require exact nested canonical output. Cover all templates with concrete model values and extend the existing scenario-fragment test that uses `model::change::fixtures::int_0017()`:

```rust
#[test]
fn standalone_statement_fragments_cover_every_template() {
    let notion = |value: &str| Sp {
        node: NotionName::new(value).unwrap(),
        span: Span::default(),
    };
    let attr = AttrRef { notion: notion("Invoice"), attr: Sp {
        node: FieldName::new("state").unwrap(), span: Span::default(),
    }};
    let free = || Action::Free("track invoices".to_string());
    let condition = Expr::Cmp {
        op: CmpOp::Eq,
        lhs: Operand::Ref(attr.clone()),
        rhs: Operand::Lit(Literal::Symbol(Sp {
            node: "open".to_string(), span: Span::default(),
        })),
    };
    let cases = vec![
        (Statement::Ubiquitous { action: free() }, "ubiquitous"),
        (Statement::EventDriven { event: notion("InvoiceIssued"), on: Some(notion("Invoice")), action: free() }, "event-driven"),
        (Statement::StateDriven { subject: attr, value: Literal::Symbol(Sp { node: "open".into(), span: Span::default() }), action: free() }, "state-driven"),
        (Statement::Unwanted { condition, action: free() }, "unwanted"),
        (Statement::Optional { feature: FieldName::new("audit").unwrap(), action: free() }, "optional"),
    ];
    for (statement, expected) in cases {
        let prefix = format!("  statement {expected} {{\n");
        assert_eq!(statement_template(&statement), expected);
        let fragment = emit_statement_fragment(&statement);
        assert!(fragment.starts_with(&prefix));
        assert!(fragment.ends_with("  }\n"));
    }
}

#[test]
fn standalone_scenario_fragment_keeps_every_step() {
    let intent = crate::model::change::fixtures::int_0017();
    let fragment = emit_scenario_fragment(&intent.scenarios[0]);
    assert!(fragment.contains("  scenario SCN-0091 "));
    assert!(fragment.contains("    given Customer"));
    assert!(fragment.contains("    when  InvoiceIssued"));
    assert!(fragment.contains("    then  Invoice.state == open"));
    assert!(fragment.ends_with("  }\n"));
}
```

- [ ] **Step 2: Run the emitter tests and verify failure**

Run: `rtk cargo test -p telos-core emit::tests::standalone_ -- --nocapture`

Expected: compilation fails because the three fragment/template functions are not defined.

- [ ] **Step 3: Expose canonical fragments through the existing emitter**

Keep the current private emitters as the single formatting implementation and add narrow wrappers:

```rust
pub fn emit_statement_fragment(statement: &Statement) -> String {
    let mut out = String::new();
    emit_statement(&mut out, statement);
    out
}

pub fn emit_scenario_fragment(scenario: &Scenario) -> String {
    let mut out = String::new();
    emit_scenario(&mut out, scenario);
    out
}

pub fn statement_template(statement: &Statement) -> &'static str {
    template(statement)
}
```

- [ ] **Step 4: Run the emitter tests and verify they pass**

Run: `rtk cargo test -p telos-core emit::tests::standalone_ -- --nocapture`

Expected: both new tests pass.

- [ ] **Step 5: Add failing view-contract assertions**

Extend the billing snapshot test to assert literal projected values:

```rust
let intent = snapshot.intents.iter().find(|item| item.id == "INT-0042").unwrap();
assert_eq!(intent.statement.template, "event-driven");
assert!(intent.statement.canonical.contains("statement event-driven"));
assert!(intent.statement.canonical.contains("system shall set Invoice.state = settled"));
assert!(intent.scenarios[0].canonical.contains("given Invoice"));
assert!(intent.scenarios[0].canonical.contains("then  Invoice.state == settled"));
```

Extend `data.rs` serialization assertions to require `snapshot.intents[0].statement.template` and `snapshot.scenarios[0].canonical`.

- [ ] **Step 6: Run the view tests and verify failure**

Run: `rtk cargo test -p telos view::model::tests::snapshot_contains_graph_relations_and_cross_references -- --nocapture`

Run: `rtk cargo test -p telos view::data::tests::data_script_has_exact_assignment_shape_and_live_metadata -- --nocapture`

Expected: compilation fails because `StatementView` and `ScenarioView.canonical` do not exist.

- [ ] **Step 7: Project canonical fragments in Rust**

Add the exact view structs and populate them while building each intent/scenario:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct StatementView {
    pub(crate) template: String,
    pub(crate) canonical: String,
}

pub(crate) struct IntentView {
    // existing fields
    pub(crate) statement: StatementView,
}

pub(crate) struct ScenarioView {
    // existing fields
    pub(crate) canonical: String,
}
```

Use `statement_template`, `emit_statement_fragment`, and `emit_scenario_fragment` directly from `telos_core::emit`.

- [ ] **Step 8: Update the TypeScript contract and strict validator test-first**

Add the types:

```ts
export type StatementTemplate =
  | 'ubiquitous'
  | 'event-driven'
  | 'state-driven'
  | 'unwanted'
  | 'optional';

export interface StatementView {
  template: StatementTemplate;
  canonical: string;
}
```

Add `statement: StatementView` to `IntentView` and `canonical: string` to `ScenarioView`. In `live.test.ts`, make the owned fixture valid with these fields, then clone it twice and assert a reload is rejected when `statement` is deleted or `scenario.canonical` is a number. Update `isIntent`/`isScenario` with strict guards.

- [ ] **Step 9: Update the development payload and run contract verification**

Add an emitter-produced statement fragment to every intent in `frontend/public/data.js`, and add the exact scenario fragment to every nested and top-level scenario. Then run:

Run: `rtk cargo test -p telos view:: -- --nocapture`

Run: `rtk npm test -- --run src/data/live.test.ts` (working directory `frontend`)

Run: `rtk npm run typecheck` (working directory `frontend`)

Expected: all commands pass.

- [ ] **Step 10: Commit the canonical view contract**

```bash
rtk git add crates/telos-core/src/emit.rs crates/telos/src/view/model.rs crates/telos/src/view/data.rs frontend/src/data/types.ts frontend/src/data/live.ts frontend/src/data/live.test.ts frontend/public/data.js
rtk git commit -m "feat(view): expose canonical intent behavior"
```

### Task 2: Shared Entity Search and Destinations

**Files:**
- Create: `frontend/src/search/entities.ts`
- Create: `frontend/src/search/entities.test.ts`
- Create: `frontend/src/search/destinations.ts`
- Create: `frontend/src/search/destinations.test.ts`
- Modify: `frontend/src/components/EntityLink.vue:19-55`

**Interfaces:**
- Consumes: `GraphNodeView`, `GraphKey`, and an optional scenario-parent ID.
- Produces: `searchEntities(nodes, query, limit?) -> GraphNodeView[]`, `shouldOpenGlobalSearch(event) -> boolean`, `shortcutLabel(platform) -> string`, and `entityDestination(entity, scenarioParent?) -> RouteLocationRaw | null`.

- [ ] **Step 1: Write failing search ranking tests**

Create fixtures that prove exact, prefix, label substring, and kind matches, stable ties, whitespace handling, and the eight-result cap:

```ts
expect(searchEntities(nodes, 'INT-0011').map((node) => node.key.id)).toEqual(['INT-0011']);
expect(searchEntities(nodes, 'adult').map((node) => node.key.id)).toEqual(['INT-0011', 'SCN-0016']);
expect(searchEntities(tenIntentNodes, 'intent')).toHaveLength(8);
expect(searchEntities(nodes, '   ')).toEqual([]);
```

Add shortcut tests proving `meta/ctrl + k` is recognized globally, `/` is recognized on the body, and `/` is ignored for input, textarea, select, button, link, and `contenteditable` targets.

- [ ] **Step 2: Run search tests and verify failure**

Run: `rtk npm test -- --run src/search/entities.test.ts` (working directory `frontend`)

Expected: FAIL because `entities.ts` does not exist.

- [ ] **Step 3: Implement stable shared ranking and keyboard guards**

Use a numeric rank and original index as the only sort keys:

```ts
function matchRank(node: GraphNodeView, needle: string): number | null {
  const id = node.key.id.toLocaleLowerCase();
  const label = node.label.toLocaleLowerCase();
  const kind = node.key.kind.toLocaleLowerCase();
  if (id === needle || label === needle) return 0;
  if (id.startsWith(needle) || label.startsWith(needle)) return 1;
  if (id.includes(needle) || label.includes(needle)) return 2;
  if (kind.includes(needle)) return 3;
  return null;
}

export function searchEntities(nodes: GraphNodeView[], query: string, limit = 8): GraphNodeView[] {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return [];
  return nodes
    .map((node, index) => ({ node, index, rank: matchRank(node, needle) }))
    .filter((entry): entry is { node: GraphNodeView; index: number; rank: number } => entry.rank !== null)
    .sort((left, right) => left.rank - right.rank || left.index - right.index)
    .slice(0, limit)
    .map((entry) => entry.node);
}
```

- [ ] **Step 4: Write failing destination tests**

Assert all eight entity kinds:

```ts
expect(entityDestination({ kind: 'intent', id: 'INT-0011' })).toEqual({
  name: 'intent-detail', params: { id: 'INT-0011' },
});
expect(entityDestination({ kind: 'scenario', id: 'SCN-0016' }, 'INT-0011')).toEqual({
  name: 'intent-detail', params: { id: 'INT-0011' }, hash: '#scenario-SCN-0016',
});
expect(entityDestination({ kind: 'code', id: 'src/pet.ts' })).toEqual({
  name: 'graph', query: { focusKind: 'code', focusId: 'src/pet.ts' },
});
```

- [ ] **Step 5: Implement one destination mapping and use it in EntityLink**

Move the existing route switch into `destinations.ts`, preserve all current anchors, and return Graph focus queries for code/test. Refactor `EntityLink.vue` to call the helper with `scenarioToIntent.value.get(entity.id)`.

- [ ] **Step 6: Run shared-search verification**

Run: `rtk npm test -- --run src/search/entities.test.ts src/search/destinations.test.ts` (working directory `frontend`)

Run: `rtk npm run typecheck` (working directory `frontend`)

Expected: all tests and type checking pass.

- [ ] **Step 7: Commit shared search primitives**

```bash
rtk git add frontend/src/search frontend/src/components/EntityLink.vue
rtk git commit -m "feat(view): add shared entity search"
```

### Task 3: Global Command Palette, Mobile Header, and Brand Palette

**Files:**
- Create: `frontend/src/components/GlobalSearch.vue`
- Modify: `frontend/src/components/AppHeader.vue:1-135`
- Modify: `frontend/src/styles/tokens.css:1-104`
- Modify: `frontend/src/styles/base.css:1-110`

**Interfaces:**
- Consumes: `searchEntities`, `entityDestination`, `snapshot.nodes`, and `scenarioToIntent`.
- Produces: a header search trigger plus modal command palette, `Ctrl/Command+K` and `/` shortcuts, and a responsive menu at 48rem.

- [ ] **Step 1: Add failing platform/shortcut unit cases**

Extend `entities.test.ts` with `shortcutLabel(platform: string) -> string` expectations:

```ts
expect(shortcutLabel('MacIntel')).toBe('⌘K');
expect(shortcutLabel('Win32')).toBe('Ctrl K');
```

Run: `rtk npm test -- --run src/search/entities.test.ts` (working directory `frontend`)

Expected: FAIL because `shortcutLabel` is missing.

- [ ] **Step 2: Implement the shortcut label and command palette behavior**

Build `GlobalSearch.vue` around a native `<dialog>` so the browser owns modal focus/inert behavior. Expose `open()` for the header trigger. On open, clear the query and active index and focus the input after `nextTick`. On keydown, prevent default for recognized global shortcuts, update the active result for ArrowUp/ArrowDown, navigate with Enter, and close on Escape. Render kind, label, and ID for each result and `No matching entities` for an unmatched non-empty query.

The core wiring must follow this shape:

```ts
const results = computed(() => searchEntities(snapshot.value.snapshot.nodes, query.value));

async function choose(node: GraphNodeView): Promise<void> {
  const parent = node.key.kind === 'scenario' ? scenarioToIntent.value.get(node.key.id) : undefined;
  const destination = entityDestination(node.key, parent);
  if (destination) await router.push(destination);
  close();
}
```

- [ ] **Step 3: Make AppHeader responsive and accessible**

Add `menuOpen`, refs for the header/menu button/first link, a route watcher that closes the menu, document Escape/pointer handlers, and cleanup in `onUnmounted`. Desktop keeps inline links. At `max-width: 48rem`, hide the inline layout, show the menu button, and position the stacked navigation below the row. Focus the first link on open and return focus to the button when Escape closes it.

Place the search trigger before `ThemeToggle`; show its label and shortcut on desktop and its icon/accessible name on mobile. Give menu/search targets a 2.5rem minimum dimension.

- [ ] **Step 4: Replace the green brand tokens with logo blue/cyan**

Use these accessible token families and apply them through existing consumers:

```css
:root {
  --color-bg-subtle: #f2f7ff;
  --color-border: #d5e1f0;
  --color-text: #14213a;
  --color-text-muted: #52637a;
  --color-primary: #0162fb;
  --color-primary-strong: #0047b8;
  --color-primary-soft: #e7f1ff;
  --color-accent: #03e1f0;
  --color-focus-ring: #0162fb;
  --k-intent: #0162fb;
  --tel-keyword: #0047b8;
}

[data-theme='dark'] {
  --color-bg: #0d1420;
  --color-bg-subtle: #131d2b;
  --color-surface: #172233;
  --color-border: #2b3b52;
  --color-text: #edf4ff;
  --color-text-muted: #a7b7cc;
  --color-primary: #69a5ff;
  --color-primary-strong: #9bc8ff;
  --color-primary-soft: #142e52;
  --color-accent: #34dce8;
  --color-focus-ring: #69a5ff;
}
```

Keep `--color-status-ok` green. Confirm no success state inherits primary blue by accident.

- [ ] **Step 5: Run header/palette verification**

Run: `rtk npm test -- --run src/search/entities.test.ts src/search/destinations.test.ts` (working directory `frontend`)

Run: `rtk npm run typecheck` (working directory `frontend`)

Run: `rtk npm run build` (working directory `frontend`)

Expected: tests, type checking, and production build pass.

- [ ] **Step 6: Commit shared navigation and palette**

```bash
rtk git add frontend/src/components/GlobalSearch.vue frontend/src/components/AppHeader.vue frontend/src/styles/tokens.css frontend/src/styles/base.css frontend/src/search/entities.ts frontend/src/search/entities.test.ts
rtk git commit -m "feat(view): add responsive global navigation"
```

### Task 4: Localized Footer Date

**Files:**
- Create: `frontend/src/format/date.ts`
- Create: `frontend/src/format/date.test.ts`
- Modify: `frontend/src/components/AppFooter.vue:1-30`

**Interfaces:**
- Produces: `formatLocalDate(value: string, locales?: Intl.LocalesArgument) -> string`.

- [ ] **Step 1: Write failing calendar-date tests**

```ts
expect(formatLocalDate('2026-08-25', 'en-GB')).toBe('25 Aug 2026');
expect(formatLocalDate('2024-02-29', 'fr-FR')).toMatch(/29.*févr.*2024/i);
expect(formatLocalDate('2026-02-29', 'en-GB')).toBe('2026-02-29');
expect(formatLocalDate('not-a-date', 'en-GB')).toBe('not-a-date');
```

- [ ] **Step 2: Run the date test and verify failure**

Run: `rtk npm test -- --run src/format/date.test.ts` (working directory `frontend`)

Expected: FAIL because `date.ts` does not exist.

- [ ] **Step 3: Implement timezone-safe local formatting**

Parse with `/^(\d{4})-(\d{2})-(\d{2})$/`, construct `new Date(year, month - 1, day)`, compare all three resulting components to reject rollover, and format with `{ dateStyle: 'medium' }` inside `try/catch`. Return the original string on every invalid path.

- [ ] **Step 4: Use the formatter in the footer**

```ts
const buildDate = computed(() => formatLocalDate(meta.value.build_date));
```

Render `built {{ buildDate }}` while leaving the version and punctuation unchanged.

- [ ] **Step 5: Run date/footer verification and commit**

Run: `rtk npm test -- --run src/format/date.test.ts` (working directory `frontend`)

Run: `rtk npm run typecheck` (working directory `frontend`)

```bash
rtk git add frontend/src/format frontend/src/components/AppFooter.vue
rtk git commit -m "feat(view): localize the build date"
```

### Task 5: Graph Finder and Shareable Focus

**Files:**
- Create: `frontend/src/graph/search.ts`
- Create: `frontend/src/graph/search.test.ts`
- Create: `frontend/src/components/GraphFinder.vue`
- Modify: `frontend/src/pages/GraphPage.vue:1-330`
- Modify: `frontend/src/graph/CytoGraph.vue:1-245`

**Interfaces:**
- Consumes: `searchEntities`, raw graph nodes, collapsed IDs, and route query values.
- Produces: `expandAncestorsForNode`, `graphFocusFromQuery`, `GraphFocusRequest { key: GraphKey; token: number }` from `graph/search.ts`, and a Cytoscape focus prop.

- [ ] **Step 1: Write failing ancestor and query tests**

```ts
expect(expandAncestorsForNode(nodes, new Set(['context:pet', 'capability:pet/care']), {
  kind: 'intent', id: 'INT-0011',
})).toEqual(new Set());

expect(graphFocusFromQuery({ focusKind: 'code', focusId: 'src/pet.ts' })).toEqual({
  kind: 'code', id: 'src/pet.ts',
});
expect(graphFocusFromQuery({ focusKind: 'unknown', focusId: 'x' })).toBeNull();
expect(graphFocusFromQuery({ focusKind: 'intent' })).toBeNull();
```

- [ ] **Step 2: Run graph-search tests and verify failure**

Run: `rtk npm test -- --run src/graph/search.test.ts` (working directory `frontend`)

Expected: FAIL because `graph/search.ts` does not exist.

- [ ] **Step 3: Implement pure expansion/query helpers**

Index nodes by `graphKeyId`, walk `node.parent` until null, and remove each container ancestor from a cloned collapsed set. Parse only the eight `GraphKeyKind` values and require scalar non-empty `focusKind`/`focusId` query strings.

- [ ] **Step 4: Implement the accessible GraphFinder**

Use `searchEntities(props.nodes, query)` and emit `choose: [node: GraphNodeView]`. Render a combobox/listbox with active-descendant IDs, arrow navigation, Enter, Escape, visible kind/label/ID, and `No matching nodes`. Keep the query after selection so the chosen entity remains evident, but close the list.

- [ ] **Step 5: Wire selection, ancestor expansion, and route focus in GraphPage**

Add:

```ts
const focusRequest = ref<GraphFocusRequest | null>(null);
let focusToken = 0;

function chooseNode(node: GraphNodeView): void {
  persistCollapsedIds(expandAncestorsForNode(rawNodes.value, collapsedIds.value, node.key));
  setSelection({ type: 'node', entity: node.key, label: node.label });
  focusRequest.value = { key: node.key, token: ++focusToken };
}
```

Watch the parsed route focus. If the key exists in `rawNodes`, call `chooseNode`; otherwise leave the current selection untouched. Place `GraphFinder` beside the relation filter and pass `focusRequest` to `CytoGraph`.

- [ ] **Step 6: Focus only after Cytoscape elements and layout are current**

Export the request interface from `graph/search.ts`, import it into both components, and add the prop:

```ts
export interface GraphFocusRequest {
  key: GraphKey;
  token: number;
}
```

Make `relayoutGraph()` return the `runCompoundLayout` promise. After each element rebuild and whenever the focus token changes, await the current layout, confirm the request token is still current, locate `nodeId(request.key)`, and call `cy.animate({ center: { eles: element }, duration: 250 })`. Missing elements are a no-op. Preserve normal selection highlighting.

- [ ] **Step 7: Run graph verification and commit**

Run: `rtk npm test -- --run src/search/entities.test.ts src/graph/search.test.ts src/graph` (working directory `frontend`)

Run: `rtk npm run typecheck` (working directory `frontend`)

Run: `rtk npm run build` (working directory `frontend`)

```bash
rtk git add frontend/src/graph frontend/src/components/GraphFinder.vue frontend/src/pages/GraphPage.vue
rtk git commit -m "feat(view): add graph node finder"
```

### Task 6: Intent Detail Canonical Parity

**Files:**
- Modify: `frontend/src/pages/IntentDetailPage.vue:1-330`

**Interfaces:**
- Consumes: `IntentView.statement`, `ScenarioView.canonical`, existing `TelCode`, graph edges, and entity destinations.
- Produces: visible statement/scenario behavior and directional intent relations.

- [ ] **Step 1: Define directional relation labels as a pure map**

Move the current generic capitalization to explicit labels inside the page module:

```ts
const RELATION_LABELS = {
  refines: { outgoing: 'Refines', incoming: 'Refined by' },
  requires: { outgoing: 'Requires', incoming: 'Required by' },
  excludes: { outgoing: 'Excludes', incoming: 'Excluded by' },
} as const;
```

Filter the Relations section to these three kinds because all other relations retain dedicated sections.

- [ ] **Step 2: Render the approved information hierarchy**

Add a back link and visible owner near the heading. Render an always-visible statement card:

```vue
<section class="statement-card" aria-labelledby="statement-heading">
  <header>
    <h2 id="statement-heading">Statement</h2>
    <span class="statement-card__template">{{ intent.statement.template }}</span>
  </header>
  <TelCode :source="intent.statement.canonical" />
</section>
```

Move the full canonical intent disclosure below scenarios. In each scenario card, render `<TelCode :source="scenario.canonical" />` before proof state so every given/when/then line is visible.

- [ ] **Step 3: Add responsive/overflow styling**

Use blue soft borders/background for the statement card, preserve horizontal scrolling inside `TelCode`, keep owner/status wrapping, and ensure scenario anchors retain the sticky-header scroll margin.

- [ ] **Step 4: Run intent verification and commit**

Run: `rtk npm test -- --run` (working directory `frontend`)

Run: `rtk npm run typecheck` (working directory `frontend`)

Run: `rtk npm run build` (working directory `frontend`)

```bash
rtk git add frontend/src/pages/IntentDetailPage.vue
rtk git commit -m "feat(view): show complete intent behavior"
```

### Task 7: Typed, Sortable Glossary Consumers

**Files:**
- Create: `frontend/src/pages/glossary-consumers.ts`
- Create: `frontend/src/pages/glossary-consumers.test.ts`
- Create: `frontend/src/components/UsedByTable.vue`
- Modify: `frontend/src/pages/GlossaryPage.vue:1-190`

**Interfaces:**
- Consumes: `notionUsedBy`, `intentById`, `scenarioById`, and `GraphKey[]` consumers.
- Produces: `ConsumerRow`, `consumerRows`, `filterConsumerRows`, `sortConsumerRows`, and the card-level table.

- [ ] **Step 1: Write failing consumer row tests**

```ts
const rows = consumerRows(
  [{ kind: 'scenario', id: 'SCN-0016' }, { kind: 'intent', id: 'INT-0011' }],
  new Map([['INT-0011', { title: 'Adults are harder to please' }]]),
  new Map([['SCN-0016', { title: 'the same game, a smaller joy' }]]),
);
expect(sortConsumerRows(rows, 'kind', 'asc').map((row) => row.id)).toEqual([
  'INT-0011', 'SCN-0016',
]);
expect(filterConsumerRows(rows, 'scenario')).toEqual([rows[0]]);
```

Also assert missing snapshot labels fall back to IDs and input arrays are not mutated.

- [ ] **Step 2: Run consumer tests and verify failure**

Run: `rtk npm test -- --run src/pages/glossary-consumers.test.ts` (working directory `frontend`)

Expected: FAIL because the consumer helper does not exist.

- [ ] **Step 3: Implement typed consumer transformations**

Define:

```ts
export type ConsumerKind = 'intent' | 'scenario';
export type ConsumerSort = 'kind' | 'id' | 'title';
export type SortDirection = 'asc' | 'desc';
export interface ConsumerRow { kind: ConsumerKind; id: string; title: string; entity: GraphKey }
```

Ignore non-intent/scenario keys defensively, resolve titles from maps, filter by selected kind, and sort copies with `localeCompare`, using ID as the stable secondary key.

- [ ] **Step 4: Build UsedByTable with accessible sortable headers**

Accept `consumers: GraphKey[]` and `kindFilter: '' | ConsumerKind`. Keep local `sort`/`direction`, toggle direction when the same header is clicked, and expose `aria-sort` on the active `<th>`. Render Type with `KindPill`, Reference as monospace ID, and Title with `EntityLink :show-kind="false"`. Wrap the table in a focusable horizontal scroller.

- [ ] **Step 5: Compose the new filter with existing glossary filters**

Add a sidebar `Used by type` select. After text and owner filtering, retain a notion only when the selected kind is empty or `notionUsedBy.get(notion.name)` contains that kind. Pass the filter to each `UsedByTable`. Preserve `No consumers recorded.` for unreferenced notions and use the existing glossary empty state when filters remove all cards.

- [ ] **Step 6: Run glossary verification and commit**

Run: `rtk npm test -- --run src/pages/page-data.test.ts src/pages/glossary-consumers.test.ts` (working directory `frontend`)

Run: `rtk npm run typecheck` (working directory `frontend`)

Run: `rtk npm run build` (working directory `frontend`)

```bash
rtk git add frontend/src/pages/GlossaryPage.vue frontend/src/pages/glossary-consumers.ts frontend/src/pages/glossary-consumers.test.ts frontend/src/components/UsedByTable.vue
rtk git commit -m "feat(view): clarify glossary consumers"
```

### Task 8: Release Version and Complete Verification

**Files:**
- Modify: `Cargo.toml:6`
- Modify: `Cargo.lock`
- Modify: `crates/telos/tests/cli_foundation.rs:28-58`
- Modify: `crates/telos-core/tests/git_oids.rs:236-397`
- Modify: `crates/telos-core/src/lock.rs:323`
- Modify: `docs/contracts.md:1471-1489`
- Modify: `frontend/public/data.js:596`

**Interfaces:**
- Consumes: all preceding implementation and the existing tag-triggered release workflow.
- Produces: workspace version 0.10.0, verified commit, annotated `v0.10.0` tag, and published GitHub release assets.

- [ ] **Step 1: Change every current-version contract to 0.10.0**

Set `[workspace.package].version = "0.10.0"`, update CLI/lock provenance expectations, update the current installation/release contract section, and set the development payload meta version to `0.10.0`. Do not rewrite historical plan/spec references.

- [ ] **Step 2: Refresh the lockfile and prove version reporting**

Run: `rtk cargo check --workspace`

Run: `rtk cargo test -p telos --test cli_foundation`

Run: `rtk cargo test -p telos-core --test git_oids -- --nocapture`

Run: `rtk cargo test -p telos-core lock::tests -- --nocapture`

Expected: every command passes and `Cargo.lock` records 0.10.0 for both workspace crates.

- [ ] **Step 3: Run the complete frontend release gate**

Run: `rtk npm test -- --run` (working directory `frontend`)

Run: `rtk npm run typecheck` (working directory `frontend`)

Run: `rtk npm run build` (working directory `frontend`)

Expected: all frontend commands pass.

- [ ] **Step 4: Run the complete Rust release gate**

Run: `rtk cargo test --workspace`

Run: `rtk cargo clippy --workspace --all-targets -- -D warnings`

Run: `rtk cargo fmt --all --check`

Expected: all workspace tests, Clippy, and formatting pass.

- [ ] **Step 5: Inspect responsive UI when a browser surface is available**

At desktop and 390px widths, verify header/menu, command palette, graph finder, intent fragments, glossary table scrolling, footer date, light/dark colors, focus order, Escape behavior, and 2.5rem targets. If the browser remains unavailable, record that limitation and rely on the mandatory unit/type/build gate.

- [ ] **Step 6: Review the final diff and commit the release preparation**

Run: `rtk git diff --check`

Run: `rtk git status --short`

Run: `rtk git diff --stat origin/main...HEAD`

```bash
rtk git add Cargo.toml Cargo.lock crates/telos/tests/cli_foundation.rs crates/telos-core/tests/git_oids.rs crates/telos-core/src/lock.rs docs/contracts.md frontend/public/data.js
rtk git commit -m "chore(release): prepare v0.10.0"
```

- [ ] **Step 7: Re-run release-critical smoke checks at the exact commit**

Run: `rtk cargo test --workspace`

Run: `rtk npm test -- --run` (working directory `frontend`)

Run: `rtk git status --short --branch`

Expected: tests pass and the worktree is clean.

- [ ] **Step 8: Tag, push, and monitor publication**

```bash
rtk git tag -a v0.10.0 -m "Telos 0.10.0"
rtk git push origin main
rtk git push origin v0.10.0
rtk gh run list --workflow Release --limit 1
rtk gh run watch --exit-status <run-id>
rtk gh release view v0.10.0 --json tagName,url,assets
```

Expected: the Release workflow succeeds and the GitHub release contains checksums plus every platform archive defined in `.github/workflows/release.yml`.
