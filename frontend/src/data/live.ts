import { reactive } from 'vue';

import { GRAPH_RELATIONS, type TelosMode, type TelosPayload } from './types';

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
    isStringArray(value.notions) &&
    isStringArray(value.proves)
  );
}

function isIntent(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.id === 'string' &&
    typeof value.title === 'string' &&
    isOneOf(value.status, ['draft', 'active', 'deprecated']) &&
    typeof value.telos === 'string' &&
    typeof value.canonical === 'string' &&
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

function isGraphKey(value: unknown): boolean {
  return (
    isRecord(value) &&
    isOneOf(value.kind, ['notion', 'intent', 'scenario', 'constraint', 'code', 'test']) &&
    typeof value.id === 'string'
  );
}

function isGraphNode(value: unknown): boolean {
  return isRecord(value) && isGraphKey(value.key) && typeof value.label === 'string';
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
    isArrayOf(snapshot.notions, isNotion) &&
    isArrayOf(snapshot.intents, isIntent) &&
    isArrayOf(snapshot.scenarios, isScenario) &&
    isArrayOf(snapshot.constraints, isConstraint) &&
    isArrayOf(snapshot.implementations, isImplementation) &&
    isArrayOf(snapshot.proofs, isProof) &&
    isArrayOf(snapshot.nodes, isGraphNode) &&
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
