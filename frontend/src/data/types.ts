// The TypeScript mirror of `ViewSnapshot` and the structs it references in
// `crates/telos/src/view/model.rs`. Field names match the Rust struct fields
// verbatim (none of them carry a serde rename), and every field's shape
// matches what `#[derive(Serialize)]` produces for it.
//
// A few fields are typed as string literal unions here even though their
// Rust field type is a plain `String` (never a serde enum): `model.rs`
// builds them from small, exhaustive `match` functions (`state_kind`,
// `drift_kind`, `intent_status`, `notion_kind`, `constraint_kind`, and
// `Relation::as_str`), so the value space really is closed. Fields built
// from genuinely free-form strings (a constraint's `scope`, an open
// change's `status`) stay `string`.

/** The bridge the host page hands to the SPA. Never fetched — always present
 * on `window` before `src/main.ts` runs, written either by `public/data.js`
 * (dev fixture) or by the `telos` binary (live/export). */
declare global {
  interface Window {
    __TELOS_DATA__?: TelosPayload;
  }
}

export type TelosMode = 'live' | 'export';

export interface TelosMeta {
  version: string;
  build_date: string;
  mode: TelosMode;
}

export interface TelosPayload {
  meta: TelosMeta;
  snapshot: ViewSnapshot;
}

// --- dashboard --------------------------------------------------------------

export type ProjectState = 'coherent' | 'changing' | 'drifted';
export type DriftKind = 'modified' | 'missing' | 'untracked';

export interface DriftView {
  path: string;
  kind: DriftKind;
}

export interface OpenChangeView {
  id: string;
  status: string;
  obligations: string[];
}

export interface DashboardView {
  state: ProjectState;
  drift: DriftView[];
  open_changes: OpenChangeView[];
}

// --- coverage -----------------------------------------------------------------

export interface CoverageRowView {
  intent: string;
  scenario: string;
  test: string | null;
}

export interface CoverageView {
  notions: number;
  constraints: number;
  intents_total: number;
  intents_active: number;
  intents_implemented: number;
  scenarios_total: number;
  scenarios_proved: number;
  rows: CoverageRowView[];
}

// --- notions ------------------------------------------------------------------

export type NotionKind = 'actor' | 'entity' | 'value' | 'event' | 'state';

export interface NotionView {
  name: string;
  kind: NotionKind;
  definition: string;
  canonical: string;
}

// --- intents & scenarios --------------------------------------------------

export type IntentStatus = 'draft' | 'active' | 'deprecated';

export interface ConstraintRefView {
  id: string;
  title: string;
  scope: string;
  canonical: string;
}

export interface ScenarioView {
  id: string;
  intent: string;
  title: string;
  notions: string[];
  proves: string[];
}

export interface IntentView {
  id: string;
  title: string;
  status: IntentStatus;
  telos: string;
  canonical: string;
  notions: string[];
  constraints: ConstraintRefView[];
  implements: string[];
  scenarios: ScenarioView[];
}

// --- constraints --------------------------------------------------------------

export type ConstraintKind = 'stack' | 'architecture' | 'quality' | 'security' | 'convention';

export interface ConstraintView {
  id: string;
  kind: ConstraintKind;
  title: string;
  scope: string;
  canonical: string;
}

// --- bindings -----------------------------------------------------------------

export interface ImplementationView {
  path: string;
  intent: string;
}

export interface ProofView {
  test: string;
  scenario: string;
}

// --- graph --------------------------------------------------------------------

export type GraphKeyKind = 'notion' | 'intent' | 'scenario' | 'constraint' | 'code' | 'test';

/** Canonical `Relation::as_str()` order in `crates/telos-core/src/graph.rs`. */
export const GRAPH_RELATIONS = [
  'refines',
  'requires',
  'excludes',
  'constrains',
  'verifies',
  'uses',
  'implements',
  'proves',
] as const;

export type GraphRelation = (typeof GRAPH_RELATIONS)[number];

/** `#[serde(tag = "kind", content = "id", rename_all = "lowercase")]`, i.e.
 * `{ "kind": "notion", "id": "Customer" }`. */
export interface GraphKey {
  kind: GraphKeyKind;
  id: string;
}

export interface GraphNodeView {
  key: GraphKey;
  label: string;
}

export interface GraphEdgeView {
  from: GraphKey;
  relation: GraphRelation;
  to: GraphKey;
}

// --- snapshot -------------------------------------------------------------

export interface ViewSnapshot {
  dashboard: DashboardView;
  coverage: CoverageView;
  notions: NotionView[];
  intents: IntentView[];
  scenarios: ScenarioView[];
  constraints: ConstraintView[];
  implementations: ImplementationView[];
  proofs: ProofView[];
  nodes: GraphNodeView[];
  edges: GraphEdgeView[];
}
