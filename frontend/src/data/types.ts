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
  owner: string;
  kind: NotionKind;
  definition: string;
  canonical: string;
}

// --- intents & scenarios --------------------------------------------------

export type IntentStatus = 'draft' | 'active' | 'deprecated';
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
  canonical: string;
  notions: string[];
  proves: string[];
}

export interface IntentView {
  id: string;
  owner: string;
  title: string;
  status: IntentStatus;
  telos: string;
  canonical: string;
  statement: StatementView;
  notions: string[];
  constraints: ConstraintRefView[];
  implements: string[];
  scenarios: ScenarioView[];
}

// --- constraints --------------------------------------------------------------

export type ConstraintKind = 'stack' | 'architecture' | 'quality' | 'security' | 'convention';

export interface ConstraintView {
  id: string;
  owner: string;
  kind: ConstraintKind;
  title: string;
  scope: string;
  canonical: string;
}

// --- bounded contexts -------------------------------------------------------

export type ContextKind = 'core' | 'supporting' | 'generic';

export interface CapabilityView {
  id: string;
  title: string;
  definition: string;
}

export interface NotionMappingView {
  from: string;
  to: string;
}

export interface ContextDependencyView {
  supplier: string;
  mappings: NotionMappingView[];
}

export interface ContextHealthView {
  intents: number;
  active_intents: number;
  scenarios: number;
  proved_scenarios: number;
}

export interface ContextView {
  id: string;
  kind: ContextKind;
  title: string;
  definition: string;
  capabilities: CapabilityView[];
  dependencies: ContextDependencyView[];
  health: ContextHealthView;
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

export type GraphKeyKind =
  | 'context'
  | 'capability'
  | 'notion'
  | 'intent'
  | 'scenario'
  | 'constraint'
  | 'code'
  | 'test';

/** Canonical `Relation::as_str()` order in `crates/telos-core/src/graph.rs`. */
export const GRAPH_RELATIONS = [
  'belongs-to',
  'depends-on',
  'maps-to',
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
  parent: GraphKey | null;
}

export interface GraphEdgeView {
  from: GraphKey;
  relation: GraphRelation;
  to: GraphKey;
}

// --- snapshot -------------------------------------------------------------

export interface ViewSnapshot {
  plans: PlanView[];
  history: ReceiptView[];
  unplanned: string[];
  dashboard: DashboardView;
  coverage: CoverageView;
  contexts: ContextView[];
  notions: NotionView[];
  intents: IntentView[];
  scenarios: ScenarioView[];
  constraints: ConstraintView[];
  implementations: ImplementationView[];
  proofs: ProofView[];
  nodes: GraphNodeView[];
  edges: GraphEdgeView[];
}

export interface PlanEvent {
  id: string;
  at: string;
  version: number;
  revision: number;
  task: string | null;
  kind: string;
  data: Record<string, unknown>;
}

export interface PlanTask {
  id: string;
  title: string;
  kind: string;
  state: 'todo' | 'in_progress' | 'blocked' | 'done' | 'cancelled';
  depends_on: string[];
  targets: string[];
  allowed_paths: string[];
  acceptance: string[];
  next_action: string;
  change: string | null;
  blocker: string | null;
  spec_delta: string;
}

export interface PlanView {
  id: string;
  title: string;
  goal: string;
  revision: number;
  version: number;
  digest: string;
  state: 'draft' | 'ready' | 'approved' | 'running' | 'paused' | 'blocked' | 'completed' | 'cancelled';
  approved: boolean;
  progress: { done: number; total: number; percent: number | null };
  current_task: string | null;
  last_activity: string;
  tasks: PlanTask[];
  brief: { summary: string; exclusions: string[]; decisions: { id: string; text: string; state: string; source: string }[]; questions: { id: string; text: string; blocking: boolean; answer: string | null }[] };
  success_criteria: string[];
  scope: string[];
  events: PlanEvent[];
}

export interface ReceiptView {
  id: string;
  plan: string;
  task: string;
  revision: number;
  at: string;
  head: string | null;
  files: { path: string; before: { oid: string; mode: string } | null; after: { oid: string; mode: string } | null }[];
  observed_files: ReceiptView["files"];
  entities: { uid: string; selector: string; previous_selector: string | null; kind: string; path: string }[];
}
