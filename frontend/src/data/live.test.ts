import { describe, expect, test, vi } from 'vitest';

import type { TelosPayload } from './types';
import {
  createLiveReloadController,
  type LiveGenerationState,
  type LiveStatus,
} from './live';

const snapshot = (version: string): TelosPayload => ({
  meta: { version, build_date: '2026-08-21', mode: 'live' },
  snapshot: {
    dashboard: { state: 'coherent', drift: [], open_changes: [] },
    coverage: {
      notions: 0,
      constraints: 0,
      intents_total: 0,
      intents_active: 0,
      intents_implemented: 0,
      scenarios_total: 0,
      scenarios_proved: 0,
      rows: [],
    },
    notions: [],
    intents: [],
    scenarios: [],
    constraints: [],
    implementations: [],
    proofs: [],
    nodes: [],
    edges: [],
  },
});

const status = (
  generation: number,
  reload_error: string | null = null,
  watcher_error: string | null = null,
): LiveStatus => ({ generation, reload_error, watcher_error });

function createScheduler() {
  let nextId = 0;
  const callbacks = new Map<number, () => void>();

  return {
    schedule(callback: () => void) {
      const id = ++nextId;
      callbacks.set(id, callback);
      return id;
    },
    cancel(id: unknown) {
      callbacks.delete(id as number);
    },
    runNext() {
      const next = callbacks.entries().next().value as [number, () => void] | undefined;
      if (!next) throw new Error('No scheduled poll');
      callbacks.delete(next[0]);
      next[1]();
    },
    get pending() {
      return callbacks.size;
    },
  };
}

function createDeferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

async function waitForScheduled(scheduler: ReturnType<typeof createScheduler>): Promise<void> {
  await vi.waitFor(() => expect(scheduler.pending).toBe(1));
}

describe('live reload controller', () => {
  test('export mode never polls', async () => {
    const scheduler = createScheduler();
    const fetchStatus = vi.fn<() => Promise<LiveStatus>>();
    const controller = createLiveReloadController({
      mode: 'export',
      fetchStatus,
      loadSnapshot: vi.fn(),
      replaceSnapshot: vi.fn(),
      schedule: scheduler.schedule,
      cancelSchedule: scheduler.cancel,
    });

    controller.start();
    await Promise.resolve();

    expect(fetchStatus).not.toHaveBeenCalled();
    expect(scheduler.pending).toBe(0);
  });

  test('an unchanged generation does not load or replace the snapshot', async () => {
    const scheduler = createScheduler();
    const fetchStatus = vi
      .fn<() => Promise<LiveStatus>>()
      .mockResolvedValueOnce(status(4))
      .mockResolvedValueOnce(status(4));
    const loadSnapshot = vi.fn();
    const replaceSnapshot = vi.fn();
    const controller = createLiveReloadController({
      mode: 'live',
      fetchStatus,
      loadSnapshot,
      replaceSnapshot,
      schedule: scheduler.schedule,
      cancelSchedule: scheduler.cancel,
    });

    controller.start();
    await waitForScheduled(scheduler);
    scheduler.runNext();
    await waitForScheduled(scheduler);
    controller.stop();

    expect(fetchStatus).toHaveBeenCalledTimes(2);
    expect(loadSnapshot).not.toHaveBeenCalled();
    expect(replaceSnapshot).not.toHaveBeenCalled();
  });

  test('a stale generation after a newer one does not load or replace', async () => {
    const scheduler = createScheduler();
    const fetchStatus = vi
      .fn<() => Promise<LiveStatus>>()
      .mockResolvedValueOnce(status(10))
      .mockResolvedValueOnce(status(9));
    const loadSnapshot = vi.fn();
    const replaceSnapshot = vi.fn();
    const controller = createLiveReloadController({
      mode: 'live',
      fetchStatus,
      loadSnapshot,
      replaceSnapshot,
      schedule: scheduler.schedule,
      cancelSchedule: scheduler.cancel,
    });

    controller.start();
    await waitForScheduled(scheduler);
    scheduler.runNext();
    await waitForScheduled(scheduler);
    controller.stop();

    expect(loadSnapshot).not.toHaveBeenCalled();
    expect(replaceSnapshot).not.toHaveBeenCalled();
  });

  test('a growing generation loads and atomically replaces once', async () => {
    const scheduler = createScheduler();
    const nextSnapshot = snapshot('next');
    const fetchStatus = vi
      .fn<() => Promise<LiveStatus>>()
      .mockResolvedValueOnce(status(7))
      .mockResolvedValueOnce(status(8));
    const loadSnapshot = vi.fn<(generation: number) => Promise<unknown>>().mockResolvedValue(nextSnapshot);
    const replaceSnapshot = vi.fn();
    const controller = createLiveReloadController({
      mode: 'live',
      fetchStatus,
      loadSnapshot,
      replaceSnapshot,
      schedule: scheduler.schedule,
      cancelSchedule: scheduler.cancel,
    });

    controller.start();
    await waitForScheduled(scheduler);
    scheduler.runNext();
    await waitForScheduled(scheduler);
    controller.stop();

    expect(loadSnapshot).toHaveBeenCalledOnce();
    expect(loadSnapshot).toHaveBeenCalledWith(8);
    expect(replaceSnapshot).toHaveBeenCalledOnce();
    expect(replaceSnapshot).toHaveBeenCalledWith(nextSnapshot);
  });

  test('a script load failure preserves the current snapshot and polling continues', async () => {
    const scheduler = createScheduler();
    const fetchStatus = vi
      .fn<() => Promise<LiveStatus>>()
      .mockResolvedValueOnce(status(1))
      .mockResolvedValueOnce(status(2));
    const replaceSnapshot = vi.fn();
    const controller = createLiveReloadController({
      mode: 'live',
      fetchStatus,
      loadSnapshot: vi.fn().mockRejectedValue(new Error('data.js failed to load')),
      replaceSnapshot,
      schedule: scheduler.schedule,
      cancelSchedule: scheduler.cancel,
    });

    controller.start();
    await waitForScheduled(scheduler);
    scheduler.runNext();
    await waitForScheduled(scheduler);

    expect(replaceSnapshot).not.toHaveBeenCalled();
    expect(controller.state.client_error).toBe('data.js failed to load');
    expect(scheduler.pending).toBe(1);
    controller.stop();
  });

  test('a failed generation is retried until it can be replaced', async () => {
    const scheduler = createScheduler();
    const nextSnapshot = snapshot('recovered');
    const fetchStatus = vi
      .fn<() => Promise<LiveStatus>>()
      .mockResolvedValueOnce(status(1))
      .mockResolvedValueOnce(status(2))
      .mockResolvedValueOnce(status(2));
    const loadSnapshot = vi
      .fn<(generation: number) => Promise<unknown>>()
      .mockRejectedValueOnce(new Error('temporary load failure'))
      .mockResolvedValueOnce(nextSnapshot);
    const replaceSnapshot = vi.fn();
    const controller = createLiveReloadController({
      mode: 'live',
      fetchStatus,
      loadSnapshot,
      replaceSnapshot,
      schedule: scheduler.schedule,
      cancelSchedule: scheduler.cancel,
    });

    controller.start();
    await waitForScheduled(scheduler);
    scheduler.runNext();
    await waitForScheduled(scheduler);
    scheduler.runNext();
    await waitForScheduled(scheduler);
    controller.stop();

    expect(loadSnapshot).toHaveBeenCalledTimes(2);
    expect(replaceSnapshot).toHaveBeenCalledOnce();
    expect(replaceSnapshot).toHaveBeenCalledWith(nextSnapshot);
  });

  test('a controller restart adopts an in-flight load and still reaches the newest generation', async () => {
    const firstScheduler = createScheduler();
    const restartedScheduler = createScheduler();
    const oldLoad = createDeferred<TelosPayload>();
    const initialSnapshot = snapshot('initial');
    const generation11 = snapshot('generation-11');
    const generation12 = snapshot('generation-12');
    let globalSnapshot = initialSnapshot;
    let visibleSnapshot = initialSnapshot;
    const generationState: LiveGenerationState = {
      last_seen: null,
      reload_required: null,
      pending_load: null,
    };
    const loadSnapshot = vi.fn((generation: number) => {
      if (generation === 11) {
        return oldLoad.promise.then((payload) => {
          globalSnapshot = payload;
          return payload;
        });
      }
      if (generation === 12) {
        globalSnapshot = generation12;
        return Promise.resolve(generation12);
      }
      return Promise.reject(new Error(`Unexpected generation ${generation}`));
    });
    const replaceSnapshot = vi.fn((payload: TelosPayload) => {
      visibleSnapshot = payload;
    });
    const firstOptions = {
      mode: 'live' as const,
      fetchStatus: vi
        .fn<() => Promise<LiveStatus>>()
        .mockResolvedValueOnce(status(10))
        .mockResolvedValueOnce(status(11)),
      loadSnapshot,
      replaceSnapshot,
      schedule: firstScheduler.schedule,
      cancelSchedule: firstScheduler.cancel,
      generationState,
    };
    const restartedOptions = {
      mode: 'live' as const,
      fetchStatus: vi.fn<() => Promise<LiveStatus>>().mockResolvedValue(status(12)),
      loadSnapshot,
      replaceSnapshot,
      schedule: restartedScheduler.schedule,
      cancelSchedule: restartedScheduler.cancel,
      generationState,
    };

    const firstController = createLiveReloadController(firstOptions);
    firstController.start();
    await waitForScheduled(firstScheduler);
    firstScheduler.runNext();
    await vi.waitFor(() => expect(loadSnapshot).toHaveBeenCalledWith(11));
    firstController.stop();

    const restartedController = createLiveReloadController(restartedOptions);
    restartedController.start();
    await vi.waitFor(() => expect(restartedOptions.fetchStatus).toHaveBeenCalledOnce());
    oldLoad.resolve(generation11);
    await waitForScheduled(restartedScheduler);
    restartedScheduler.runNext();
    await waitForScheduled(restartedScheduler);
    restartedController.stop();

    expect(loadSnapshot.mock.calls.map(([generation]) => generation)).toEqual([11, 12]);
    expect(replaceSnapshot).toHaveBeenCalledOnce();
    expect(visibleSnapshot).toBe(generation12);
    expect(globalSnapshot).toBe(generation12);
  });

  test('server errors are reflected and cleared by the next response', async () => {
    const scheduler = createScheduler();
    const fetchStatus = vi
      .fn<() => Promise<LiveStatus>>()
      .mockResolvedValueOnce(status(3, 'parse failed', 'watch failed'))
      .mockResolvedValueOnce(status(3));
    const controller = createLiveReloadController({
      mode: 'live',
      fetchStatus,
      loadSnapshot: vi.fn(),
      replaceSnapshot: vi.fn(),
      schedule: scheduler.schedule,
      cancelSchedule: scheduler.cancel,
    });

    controller.start();
    await waitForScheduled(scheduler);
    expect(controller.state.reload_error).toBe('parse failed');
    expect(controller.state.watcher_error).toBe('watch failed');

    scheduler.runNext();
    await waitForScheduled(scheduler);

    expect(controller.state.reload_error).toBeNull();
    expect(controller.state.watcher_error).toBeNull();
    controller.stop();
  });

  test('stop cancels the scheduled next poll', async () => {
    const scheduler = createScheduler();
    const fetchStatus = vi.fn<() => Promise<LiveStatus>>().mockResolvedValue(status(9));
    const controller = createLiveReloadController({
      mode: 'live',
      fetchStatus,
      loadSnapshot: vi.fn(),
      replaceSnapshot: vi.fn(),
      schedule: scheduler.schedule,
      cancelSchedule: scheduler.cancel,
    });

    controller.start();
    await waitForScheduled(scheduler);
    controller.stop();

    expect(scheduler.pending).toBe(0);
    expect(fetchStatus).toHaveBeenCalledOnce();
  });

  test('invalid status and network failures are reported without ending future polls', async () => {
    const scheduler = createScheduler();
    const fetchStatus = vi
      .fn<() => Promise<unknown>>()
      .mockResolvedValueOnce({ generation: 'bad', reload_error: null, watcher_error: null })
      .mockRejectedValueOnce(new Error('offline'))
      .mockResolvedValueOnce(status(5));
    const controller = createLiveReloadController({
      mode: 'live',
      fetchStatus,
      loadSnapshot: vi.fn(),
      replaceSnapshot: vi.fn(),
      schedule: scheduler.schedule,
      cancelSchedule: scheduler.cancel,
    });

    controller.start();
    await waitForScheduled(scheduler);
    expect(controller.state.client_error).toContain('Invalid live status');

    scheduler.runNext();
    await waitForScheduled(scheduler);
    expect(controller.state.client_error).toBe('offline');

    scheduler.runNext();
    await waitForScheduled(scheduler);
    expect(controller.state.client_error).toBeNull();
    controller.stop();
  });

  test.each([
    ['dashboard fields', (payload: any) => (payload.snapshot.dashboard = {})],
    [
      'dashboard drift entries',
      (payload: any) => (payload.snapshot.dashboard.drift = [{ path: 42, kind: 'modified' }]),
    ],
    [
      'dashboard open-change entries',
      (payload: any) =>
        (payload.snapshot.dashboard.open_changes = [
          { id: 'CHG-1', status: 'open', obligations: [false] },
        ]),
    ],
    ['coverage fields', (payload: any) => (payload.snapshot.coverage = {})],
    [
      'coverage rows',
      (payload: any) =>
        (payload.snapshot.coverage.rows = [{ intent: 'INT-1', scenario: null, test: null }]),
    ],
    [
      'notion entries',
      (payload: any) =>
        (payload.snapshot.notions = [
          { name: 'Customer', kind: 'entity', definition: 'A customer', canonical: 3 },
        ]),
    ],
    [
      'intent nested constraint entries',
      (payload: any) =>
        (payload.snapshot.intents = [
          {
            id: 'INT-1',
            title: 'Intent',
            status: 'active',
            telos: 'intent INT-1',
            canonical: 'intent INT-1',
            notions: ['Customer'],
            constraints: [{ id: 'C-1', title: 'Constraint', scope: 'all', canonical: 4 }],
            implements: [],
            scenarios: [],
          },
        ]),
    ],
    [
      'intent nested scenario entries',
      (payload: any) =>
        (payload.snapshot.intents = [
          {
            id: 'INT-1',
            title: 'Intent',
            status: 'active',
            telos: 'intent INT-1',
            canonical: 'intent INT-1',
            notions: [],
            constraints: [],
            implements: [],
            scenarios: [
              { id: 'SCN-1', intent: 'INT-1', title: 'Scenario', notions: [], proves: [9] },
            ],
          },
        ]),
    ],
    [
      'scenario entries',
      (payload: any) =>
        (payload.snapshot.scenarios = [
          { id: 'SCN-1', intent: 'INT-1', title: 'Scenario', notions: [null], proves: [] },
        ]),
    ],
    [
      'constraint entries',
      (payload: any) =>
        (payload.snapshot.constraints = [
          { id: 'C-1', kind: 'unknown', title: 'Constraint', scope: 'all', canonical: '' },
        ]),
    ],
    [
      'implementation entries',
      (payload: any) => (payload.snapshot.implementations = [{ path: 7, intent: 'INT-1' }]),
    ],
    ['proof entries', (payload: any) => (payload.snapshot.proofs = [{ test: null, scenario: 'S' }])],
    [
      'graph node entries',
      (payload: any) =>
        (payload.snapshot.nodes = [{ key: { kind: 'unknown', id: 'N' }, label: 'Node' }]),
    ],
    [
      'graph edge entries',
      (payload: any) =>
        (payload.snapshot.edges = [
          {
            from: { kind: 'intent', id: 'I' },
            relation: 'unknown',
            to: { kind: 'notion', id: 'N' },
          },
        ]),
    ],
  ])('a malformed payload with invalid %s preserves the prior snapshot', async (_name, mutate) => {
    const scheduler = createScheduler();
    const initialSnapshot = snapshot('initial');
    const malformed = structuredClone(snapshot('malformed')) as any;
    mutate(malformed);
    let visibleSnapshot = initialSnapshot;
    const replaceSnapshot = vi.fn((payload: TelosPayload) => {
      visibleSnapshot = payload;
    });
    const controller = createLiveReloadController({
      mode: 'live',
      fetchStatus: vi
        .fn<() => Promise<LiveStatus>>()
        .mockResolvedValueOnce(status(1))
        .mockResolvedValueOnce(status(2)),
      loadSnapshot: vi.fn().mockResolvedValue(malformed),
      replaceSnapshot,
      schedule: scheduler.schedule,
      cancelSchedule: scheduler.cancel,
    });

    controller.start();
    await waitForScheduled(scheduler);
    scheduler.runNext();
    await waitForScheduled(scheduler);
    controller.stop();

    expect(replaceSnapshot).not.toHaveBeenCalled();
    expect(visibleSnapshot).toBe(initialSnapshot);
    expect(controller.state.client_error).toBe('Invalid data.js payload');
  });
});
