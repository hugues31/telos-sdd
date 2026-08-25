import { reactive } from 'vue';

import {
  GRAPH_RELATIONS,
  type GraphKey,
  type GraphNodeView,
  type TelosMode,
  type TelosPayload,
  type ViewSnapshot,
} from './types';

export interface LiveStatus {
  generation: number;
  reload_error: string | null;
  watcher_error: string | null;
}

export interface LiveState {
  reload_error: string | null;
  watcher_error: string | null;
  client_error: string | null;
}

type TimerHandle = ReturnType<typeof setTimeout>;

interface LiveReloadOptions {
  mode: TelosMode;
  fetchStatus: () => Promise<unknown>;
  loadSnapshot: (generation: number) => Promise<unknown>;
  replaceSnapshot: (payload: TelosPayload) => void;
  schedule?: (callback: () => void, delay: number) => unknown;
  cancelSchedule?: (handle: unknown) => void;
  intervalMs?: number;
  state?: LiveState;
  generationState?: LiveGenerationState;
}

export interface LiveReloadController {
  readonly state: LiveState;
  start: () => void;
  stop: () => void;
}

interface PendingLoad {
  generation: number;
  promise: Promise<unknown>;
}

export interface LiveGenerationState {
  /** Most recent valid live.json generation; a decrease starts a new server lifecycle. */
  last_seen: number | null;
  /** Newest observed generation that has not yet been validated and swapped. */
  reload_required: number | null;
  /** Script promise retained so a replacement controller can adopt it. */
  pending_load: PendingLoad | null;
}

const createState = (): LiveState =>
  reactive({
    reload_error: null,
    watcher_error: null,
    client_error: null,
  });

const createGenerationState = (): LiveGenerationState => ({
  last_seen: null,
  reload_required: null,
  pending_load: null,
});

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((entry) => typeof entry === 'string');
}

function isArrayOf(value: unknown, guard: (entry: unknown) => boolean): boolean {
  return Array.isArray(value) && value.every(guard);
}

function isOneOf(value: unknown, choices: readonly string[]): value is string {
  return typeof value === 'string' && choices.includes(value);
}

function isCount(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

function isLiveStatus(value: unknown): value is LiveStatus {
  return (
    isRecord(value) &&
    Number.isSafeInteger(value.generation) &&
    (value.generation as number) >= 0 &&
    isNullableString(value.reload_error) &&
    isNullableString(value.watcher_error)
  );
}

function isDrift(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.path === 'string' &&
    isOneOf(value.kind, ['modified', 'missing', 'untracked'])
  );
}

function isOpenChange(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.id === 'string' &&
    typeof value.status === 'string' &&
    isStringArray(value.obligations)
  );
}

function isDashboard(value: unknown): boolean {
  return (
    isRecord(value) &&
    isOneOf(value.state, ['coherent', 'changing', 'drifted']) &&
    isArrayOf(value.drift, isDrift) &&
    isArrayOf(value.open_changes, isOpenChange)
  );
}

function isCoverageRow(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.intent === 'string' &&
    typeof value.scenario === 'string' &&
    isNullableString(value.test)
  );
}

function isCoverage(value: unknown): boolean {
  if (!isRecord(value)) return false;
  return (
    [
      value.notions,
      value.constraints,
      value.intents_total,
      value.intents_active,
      value.intents_implemented,
      value.scenarios_total,
      value.scenarios_proved,
    ].every(isCount) && isArrayOf(value.rows, isCoverageRow)
  );
}

function isNotion(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.name === 'string' &&
    typeof value.owner === 'string' &&
    isOneOf(value.kind, ['actor', 'entity', 'value', 'event', 'state']) &&
    typeof value.definition === 'string' &&
    typeof value.canonical === 'string'
  );
}

function isConstraintRef(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.id === 'string' &&
    typeof value.title === 'string' &&
    typeof value.scope === 'string' &&
    typeof value.canonical === 'string'
  );
}

function isScenario(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.id === 'string' &&
    typeof value.intent === 'string' &&
    typeof value.title === 'string' &&
    typeof value.canonical === 'string' &&
    isStringArray(value.notions) &&
    isStringArray(value.proves)
  );
}

function isStatement(value: unknown): boolean {
  return (
    isRecord(value) &&
    isOneOf(value.template, [
      'ubiquitous',
      'event-driven',
      'state-driven',
      'unwanted',
      'optional',
    ]) &&
    typeof value.canonical === 'string'
  );
}

function isContext(value: unknown): boolean {
  if (!isRecord(value)) return false;
  const health = value.health;
  return (
    typeof value.id === 'string' &&
    isOneOf(value.kind, ['core', 'supporting', 'generic']) &&
    typeof value.title === 'string' &&
    typeof value.definition === 'string' &&
    isArrayOf(
      value.capabilities,
      (capability) =>
        isRecord(capability) &&
        typeof capability.id === 'string' &&
        typeof capability.title === 'string' &&
        typeof capability.definition === 'string',
    ) &&
    isArrayOf(
      value.dependencies,
      (dependency) =>
        isRecord(dependency) &&
        typeof dependency.supplier === 'string' &&
        isArrayOf(
          dependency.mappings,
          (mapping) =>
            isRecord(mapping) &&
            typeof mapping.from === 'string' &&
            typeof mapping.to === 'string',
        ),
    ) &&
    isRecord(health) &&
    [health.intents, health.active_intents, health.scenarios, health.proved_scenarios].every(
      isCount,
    )
  );
}

function isIntent(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.id === 'string' &&
    typeof value.owner === 'string' &&
    typeof value.title === 'string' &&
    isOneOf(value.status, ['draft', 'active', 'deprecated']) &&
    typeof value.telos === 'string' &&
    typeof value.canonical === 'string' &&
    isStatement(value.statement) &&
    isStringArray(value.notions) &&
    isArrayOf(value.constraints, isConstraintRef) &&
    isStringArray(value.implements) &&
    isArrayOf(value.scenarios, isScenario)
  );
}

function isConstraint(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.id === 'string' &&
    typeof value.owner === 'string' &&
    isOneOf(value.kind, ['stack', 'architecture', 'quality', 'security', 'convention']) &&
    typeof value.title === 'string' &&
    typeof value.scope === 'string' &&
    typeof value.canonical === 'string'
  );
}

function isImplementation(value: unknown): boolean {
  return isRecord(value) && typeof value.path === 'string' && typeof value.intent === 'string';
}

function isProof(value: unknown): boolean {
  return isRecord(value) && typeof value.test === 'string' && typeof value.scenario === 'string';
}

function isGraphKey(value: unknown): value is GraphKey {
  return (
    isRecord(value) &&
    isOneOf(value.kind, [
      'context',
      'capability',
      'notion',
      'intent',
      'scenario',
      'constraint',
      'code',
      'test',
    ]) &&
    typeof value.id === 'string'
  );
}

function isGraphNode(value: unknown): value is GraphNodeView {
  if (!isRecord(value) || typeof value.label !== 'string') return false;

  const key = value.key;
  if (!isGraphKey(key)) return false;

  const parent = value.parent;
  const hasContainerParent =
    isGraphKey(parent) && (parent.kind === 'context' || parent.kind === 'capability');
  switch (key.kind) {
    case 'context':
    case 'code':
    case 'test':
      return parent === null;
    case 'capability':
      return isGraphKey(parent) && parent.kind === 'context';
    case 'notion':
    case 'intent':
    case 'scenario':
      return hasContainerParent;
    case 'constraint':
      return parent === null || hasContainerParent;
  }

  return false;
}

function graphKeyId(value: GraphKey): string {
  return `${value.kind}:${value.id}`;
}

function graphParentsExist(nodes: GraphNodeView[]): boolean {
  const ids = new Set(nodes.map((node) => graphKeyId(node.key)));

  return nodes.every((node) => node.parent === null || ids.has(graphKeyId(node.parent)));
}

function sameGraphKey(left: GraphKey | null, right: GraphKey | null): boolean {
  return left === null
    ? right === null
    : right !== null && left.kind === right.kind && left.id === right.id;
}

function graphHierarchyMatchesSnapshot(snapshot: ViewSnapshot): boolean {
  const expectedParents = new Map<string, GraphKey | null>();
  const ownerKeys = new Map<string, GraphKey>();

  function addExpected(key: GraphKey, parent: GraphKey | null): boolean {
    const id = graphKeyId(key);
    if (expectedParents.has(id)) return false;
    expectedParents.set(id, parent);
    return true;
  }

  for (const context of snapshot.contexts) {
    const contextKey: GraphKey = { kind: 'context', id: context.id };
    if (!addExpected(contextKey, null) || ownerKeys.has(context.id)) return false;
    ownerKeys.set(context.id, contextKey);
  }
  for (const context of snapshot.contexts) {
    const contextKey = ownerKeys.get(context.id);
    if (!contextKey) return false;
    for (const capability of context.capabilities) {
      const capabilityKey: GraphKey = { kind: 'capability', id: capability.id };
      if (!addExpected(capabilityKey, contextKey) || ownerKeys.has(capability.id)) return false;
      ownerKeys.set(capability.id, capabilityKey);
    }
  }

  function parentForOwner(owner: string): GraphKey | null | undefined {
    if (owner === 'project') return null;
    return ownerKeys.get(owner);
  }

  for (const notion of snapshot.notions) {
    const parent = parentForOwner(notion.owner);
    if (parent === undefined || !addExpected({ kind: 'notion', id: notion.name }, parent)) {
      return false;
    }
  }

  const intentOwner = new Map<string, GraphKey>();
  for (const intent of snapshot.intents) {
    const parent = parentForOwner(intent.owner);
    if (
      parent === undefined ||
      parent === null ||
      intentOwner.has(intent.id) ||
      !addExpected({ kind: 'intent', id: intent.id }, parent)
    ) {
      return false;
    }
    intentOwner.set(intent.id, parent);
  }

  for (const scenario of snapshot.scenarios) {
    const parent = intentOwner.get(scenario.intent);
    if (!parent || !addExpected({ kind: 'scenario', id: scenario.id }, parent)) return false;
  }

  for (const constraint of snapshot.constraints) {
    const parent = parentForOwner(constraint.owner);
    if (parent === undefined || !addExpected({ kind: 'constraint', id: constraint.id }, parent)) {
      return false;
    }
  }

  const seen = new Set<string>();
  for (const node of snapshot.nodes) {
    const id = graphKeyId(node.key);
    if (seen.has(id)) return false;
    seen.add(id);

    if (node.key.kind === 'code' || node.key.kind === 'test') {
      if (node.parent !== null) return false;
      continue;
    }

    if (!expectedParents.has(id)) return false;
    if (!sameGraphKey(node.parent, expectedParents.get(id) ?? null)) return false;
  }

  return [...expectedParents.keys()].every((id) => seen.has(id));
}

function isGraphEdge(value: unknown): boolean {
  return (
    isRecord(value) &&
    isGraphKey(value.from) &&
    isOneOf(value.relation, GRAPH_RELATIONS) &&
    isGraphKey(value.to)
  );
}

function isTelosPayload(value: unknown): value is TelosPayload {
  if (!isRecord(value) || !isRecord(value.meta) || !isRecord(value.snapshot)) return false;

  const { meta, snapshot } = value;
  if (
    typeof meta.version !== 'string' ||
    typeof meta.build_date !== 'string' ||
    (meta.mode !== 'live' && meta.mode !== 'export')
  ) {
    return false;
  }

  return (
    isDashboard(snapshot.dashboard) &&
    isCoverage(snapshot.coverage) &&
    isArrayOf(snapshot.contexts, isContext) &&
    isArrayOf(snapshot.notions, isNotion) &&
    isArrayOf(snapshot.intents, isIntent) &&
    isArrayOf(snapshot.scenarios, isScenario) &&
    isArrayOf(snapshot.constraints, isConstraint) &&
    isArrayOf(snapshot.implementations, isImplementation) &&
    isArrayOf(snapshot.proofs, isProof) &&
    isArrayOf(snapshot.nodes, isGraphNode) &&
    graphParentsExist(snapshot.nodes as GraphNodeView[]) &&
    graphHierarchyMatchesSnapshot(snapshot as unknown as ViewSnapshot) &&
    isArrayOf(snapshot.edges, isGraphEdge)
  );
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function createLiveReloadController(options: LiveReloadOptions): LiveReloadController {
  const state = options.state ?? createState();
  const schedule =
    options.schedule ?? ((callback: () => void, delay: number) => setTimeout(callback, delay));
  const cancelSchedule =
    options.cancelSchedule ?? ((handle: unknown) => clearTimeout(handle as TimerHandle));
  const intervalMs = options.intervalMs ?? 1_000;
  const generationState = options.generationState ?? createGenerationState();

  let timer: unknown;
  let running = false;
  let runId = 0;
  let needsSynchronization = true;

  const poll = async (currentRun: number): Promise<void> => {
    let adoptedLoad: PendingLoad | null = null;
    try {
      const rawStatus = await options.fetchStatus();
      if (currentRun !== runId || !running) return;
      if (!isLiveStatus(rawStatus)) throw new Error('Invalid live status response');

      state.reload_error = rawStatus.reload_error;
      state.watcher_error = rawStatus.watcher_error;
      state.client_error = null;

      if (
        needsSynchronization ||
        generationState.last_seen === null ||
        rawStatus.generation !== generationState.last_seen
      ) {
        generationState.last_seen = rawStatus.generation;
        generationState.reload_required = rawStatus.generation;
      }

      while (generationState.reload_required !== null) {
        const pending =
          generationState.pending_load ??
          (generationState.pending_load = {
            generation: generationState.reload_required,
            promise: options.loadSnapshot(generationState.reload_required),
          });
        adoptedLoad = pending;
        const nextSnapshot = await pending.promise;
        if (currentRun !== runId || !running) return;
        if (generationState.pending_load === pending) generationState.pending_load = null;
        if (generationState.reload_required !== pending.generation) continue;
        if (!isTelosPayload(nextSnapshot)) throw new Error('Invalid data.js payload');
        options.replaceSnapshot(nextSnapshot);
        generationState.reload_required = null;
        needsSynchronization = false;
      }
    } catch (error) {
      if (currentRun === runId && running) {
        if (generationState.pending_load === adoptedLoad) generationState.pending_load = null;
        needsSynchronization = true;
        state.client_error = errorMessage(error);
      }
    } finally {
      if (currentRun === runId && running) {
        timer = schedule(() => {
          timer = undefined;
          void poll(currentRun);
        }, intervalMs);
      }
    }
  };

  return {
    state,
    start() {
      if (running || options.mode !== 'live') return;
      running = true;
      needsSynchronization = true;
      const currentRun = ++runId;
      void poll(currentRun);
    },
    stop() {
      running = false;
      runId += 1;
      if (timer !== undefined) {
        cancelSchedule(timer);
        timer = undefined;
      }
    },
  };
}

async function fetchLiveStatus(): Promise<unknown> {
  const response = await fetch('/live.json', { cache: 'no-store' });
  if (!response.ok) throw new Error(`Live status request failed (${response.status})`);
  return response.json();
}

function loadDataScript(generation: number): Promise<unknown> {
  const previous = window.__TELOS_DATA__;
  window.__TELOS_DATA__ = undefined;

  return new Promise((resolve, reject) => {
    const script = document.createElement('script');
    script.async = false;
    script.src = `/data.js?g=${generation}`;

    const finish = () => script.remove();
    script.addEventListener('load', () => {
      const next = window.__TELOS_DATA__;
      finish();
      if (!isTelosPayload(next)) {
        window.__TELOS_DATA__ = previous;
        reject(new Error('Invalid data.js payload'));
        return;
      }
      resolve(next);
    });
    script.addEventListener('error', () => {
      finish();
      window.__TELOS_DATA__ = previous;
      reject(new Error(`Failed to load /data.js?g=${generation}`));
    });
    document.head.append(script);
  });
}

export const liveState = createState();

const hotData = import.meta.hot?.data as { generationState?: LiveGenerationState } | undefined;
// App remounts and live.ts HMR both retain an in-flight script obligation.
const browserGenerationState = hotData?.generationState ?? createGenerationState();

let activeController: LiveReloadController | null = null;

export function startLiveReload(
  mode: TelosMode,
  swapSnapshot: (payload: TelosPayload) => void,
): () => void {
  activeController?.stop();
  liveState.reload_error = null;
  liveState.watcher_error = null;
  liveState.client_error = null;

  const controller = createLiveReloadController({
    mode,
    fetchStatus: fetchLiveStatus,
    loadSnapshot: loadDataScript,
    replaceSnapshot: swapSnapshot,
    state: liveState,
    generationState: browserGenerationState,
  });
  activeController = controller;
  controller.start();

  return () => {
    controller.stop();
    if (activeController === controller) activeController = null;
  };
}

if (import.meta.hot) {
  import.meta.hot.dispose((data) => {
    data.generationState = browserGenerationState;
    activeController?.stop();
    activeController = null;
  });
}
