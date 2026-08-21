import { reactive } from 'vue';

import type { TelosMode, TelosPayload } from './types';

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
}

export interface LiveReloadController {
  readonly state: LiveState;
  start: () => void;
  stop: () => void;
}

const createState = (): LiveState =>
  reactive({
    reload_error: null,
    watcher_error: null,
    client_error: null,
  });

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
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

function isTelosPayload(value: unknown): value is TelosPayload {
  if (!isRecord(value) || !isRecord(value.meta) || !isRecord(value.snapshot)) return false;

  const { meta, snapshot } = value;
  if (
    typeof meta.version !== 'string' ||
    typeof meta.build_date !== 'string' ||
    (meta.mode !== 'live' && meta.mode !== 'export') ||
    !isRecord(snapshot.dashboard) ||
    !isRecord(snapshot.coverage)
  ) {
    return false;
  }

  return [
    'notions',
    'intents',
    'scenarios',
    'constraints',
    'implementations',
    'proofs',
    'nodes',
    'edges',
  ].every((key) => Array.isArray(snapshot[key]));
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

  let timer: unknown;
  let running = false;
  let runId = 0;
  let lastGeneration: number | null = null;

  const poll = async (currentRun: number): Promise<void> => {
    try {
      const rawStatus = await options.fetchStatus();
      if (currentRun !== runId || !running) return;
      if (!isLiveStatus(rawStatus)) throw new Error('Invalid live status response');

      state.reload_error = rawStatus.reload_error;
      state.watcher_error = rawStatus.watcher_error;
      state.client_error = null;

      if (lastGeneration === null) {
        lastGeneration = rawStatus.generation;
      } else if (rawStatus.generation !== lastGeneration) {
        const generation = rawStatus.generation;
        const nextSnapshot = await options.loadSnapshot(generation);
        if (currentRun !== runId || !running) return;
        if (!isTelosPayload(nextSnapshot)) throw new Error('Invalid data.js payload');
        options.replaceSnapshot(nextSnapshot);
        lastGeneration = generation;
      }
    } catch (error) {
      if (currentRun === runId && running) state.client_error = errorMessage(error);
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
  });
  activeController = controller;
  controller.start();

  return () => {
    controller.stop();
    if (activeController === controller) activeController = null;
  };
}

if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    activeController?.stop();
    activeController = null;
  });
}
