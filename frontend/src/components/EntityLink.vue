<script setup lang="ts">
// Link to an entity's page, derived purely from its GraphKey. Notion,
// intent, constraint and scenario each resolve to a route (scenario needs
// `scenarioToIntent` from the data layer to find its parent intent, since
// scenarios don't have their own page — they're an anchor on the intent
// detail page). code/test have no page at all, so they render as a plain,
// same-looking, non-interactive span with an explanatory title.
//
// Label: a notion's id *is* its readable name (e.g. "Invoice"), and
// constraint ids aren't in scope for this — but an intent/scenario id
// (INT-0042, SCN-0107) means nothing on its own, so those two kinds resolve
// a human title from the snapshot computeds and fall back to the raw id
// when the entity isn't found (e.g. a dangling reference). This lives here,
// not per call-site, so every EntityLink in the app shows the same label
// for the same GraphKey.
import { computed } from 'vue';
import type { RouteLocationRaw } from 'vue-router';

import { intentById, scenarioById, scenarioToIntent } from '../data/snapshot';
import type { GraphKey } from '../data/types';
import KindPill from './KindPill.vue';

const props = withDefaults(
  defineProps<{
    entity: GraphKey;
    /** Show the KindPill alongside the id/name. Defaults to true. */
    showKind?: boolean;
  }>(),
  { showKind: true },
);

const to = computed<RouteLocationRaw | null>(() => {
  switch (props.entity.kind) {
    case 'context':
      return { name: 'contexts', hash: `#context-${props.entity.id}` };
    case 'capability':
      return { name: 'contexts', hash: `#capability-${props.entity.id.replace('/', '-')}` };
    case 'intent':
      return { name: 'intent-detail', params: { id: props.entity.id } };
    case 'scenario': {
      const intentParent = scenarioToIntent.value.get(props.entity.id);
      return intentParent
        ? { name: 'intent-detail', params: { id: intentParent }, hash: `#scenario-${props.entity.id}` }
        : null;
    }
    case 'notion':
      return { name: 'glossary', hash: `#notion-${props.entity.id}` };
    case 'constraint':
      return { name: 'coverage', hash: `#constraint-${props.entity.id}` };
    default:
      return null;
  }
});

const staticTitle = computed(() => {
  if (props.entity.kind === 'code') return 'Source file — has no dedicated page';
  if (props.entity.kind === 'test') return 'Test — has no dedicated page';
  return `${props.entity.kind} not found in this snapshot`;
});

const resolvedLabel = computed(() => {
  if (props.entity.kind === 'intent') return intentById.value.get(props.entity.id)?.title;
  if (props.entity.kind === 'scenario') return scenarioById.value.get(props.entity.id)?.title;
  return undefined;
});

const displayLabel = computed(() => resolvedLabel.value ?? props.entity.id);
</script>

<template>
  <RouterLink v-if="to" :to="to" class="entity-link" :title="resolvedLabel ? entity.id : undefined">
    <KindPill v-if="showKind" :kind="entity.kind" />
    <span class="entity-link__id" :class="{ 'entity-link__id--label': resolvedLabel }">{{ displayLabel }}</span>
  </RouterLink>
  <span v-else class="entity-link entity-link--static" :title="staticTitle">
    <KindPill v-if="showKind" :kind="entity.kind" />
    <span class="entity-link__id" :class="{ 'entity-link__id--label': resolvedLabel }">{{ displayLabel }}</span>
  </span>
</template>

<style scoped>
.entity-link {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  color: var(--color-text);
}

.entity-link--static {
  color: var(--color-text-muted);
  cursor: help;
}

.entity-link__id {
  font-family:
    ui-monospace,
    SFMono-Regular,
    Menlo,
    Consolas,
    'Liberation Mono',
    monospace;
  font-size: 0.875rem;
}

/* A resolved title (intent/scenario) is prose, not an id — drop the
   monospace treatment so it reads like a label, not a code token. */
.entity-link__id--label {
  font-family: inherit;
  font-size: inherit;
}
</style>
