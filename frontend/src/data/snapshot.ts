// The data layer: a composable module (no Pinia) that owns the one payload
// the whole SPA reads from. `window.__TELOS_DATA__` is written before this
// module loads (see index.html), so it is read once, synchronously, here.

import { computed, shallowRef } from 'vue';
import type {
  ConstraintView,
  IntentView,
  NotionView,
  TelosPayload,
} from './types';

/**
 * Whether `window.__TELOS_DATA__` was present when this module loaded.
 * `src/main.ts` checks this before mounting the app and renders a minimal
 * error page instead when it is false — an abnormal case (a broken export,
 * or index.html opened without its data.js), never a normal empty project.
 */
export const hasSnapshot = window.__TELOS_DATA__ != null;

// Cast is safe under the `hasSnapshot` contract above: nothing reads
// `payload.value` unless the app actually mounted, which only happens when
// `hasSnapshot` is true.
const payload = shallowRef<TelosPayload>(window.__TELOS_DATA__ as TelosPayload);

/** Read-only view of the current payload. Use {@link replaceSnapshot} to swap it. */
export const snapshot = computed(() => payload.value);

/** Swaps the whole payload in one go — the hook the live-reload path (a later task) will call. */
export function replaceSnapshot(next: TelosPayload): void {
  payload.value = next;
}

export const intentById = computed(() => {
  const map = new Map<string, IntentView>();
  for (const intent of payload.value.snapshot.intents) {
    map.set(intent.id, intent);
  }
  return map;
});

export const notionByName = computed(() => {
  const map = new Map<string, NotionView>();
  for (const notion of payload.value.snapshot.notions) {
    map.set(notion.name, notion);
  }
  return map;
});

export const scenarioToIntent = computed(() => {
  const map = new Map<string, string>();
  for (const scenario of payload.value.snapshot.scenarios) {
    map.set(scenario.id, scenario.intent);
  }
  return map;
});

export const constraintById = computed(() => {
  const map = new Map<string, ConstraintView>();
  for (const constraint of payload.value.snapshot.constraints) {
    map.set(constraint.id, constraint);
  }
  return map;
});

/**
 * Notion name -> ids of the entities that use it.
 *
 * `model.rs` has no direct "used by" field on `NotionView`; the only place
 * that relation is materialized is the graph, whose edges already carry a
 * `uses` relation from an intent or a scenario to a notion (see
 * `uses_from` in `view/model.rs` — those are the only two node kinds it
 * ever calls). So this is derived from `snapshot.edges`, keyed by the
 * notion (`edge.to`) and collecting the user ids (`edge.from.id`).
 */
export const notionUsedBy = computed(() => {
  const map = new Map<string, string[]>();
  for (const edge of payload.value.snapshot.edges) {
    if (edge.relation !== 'uses' || edge.to.kind !== 'notion') continue;
    const users = map.get(edge.to.id);
    if (users) {
      users.push(edge.from.id);
    } else {
      map.set(edge.to.id, [edge.from.id]);
    }
  }
  return map;
});
