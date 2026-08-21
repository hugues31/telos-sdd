<script setup lang="ts">
// Link to an entity's page, derived purely from its GraphKey. Notion,
// intent, constraint and scenario each resolve to a route (scenario needs
// `scenarioToIntent` from the data layer to find its parent intent, since
// scenarios don't have their own page — they're an anchor on the intent
// detail page). code/test have no page at all, so they render as a plain,
// same-looking, non-interactive span with an explanatory title.
import { computed } from 'vue';
import type { RouteLocationRaw } from 'vue-router';

import { scenarioToIntent } from '../data/snapshot';
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
</script>

<template>
  <RouterLink v-if="to" :to="to" class="entity-link">
    <KindPill v-if="showKind" :kind="entity.kind" />
    <span class="entity-link__id">{{ entity.id }}</span>
  </RouterLink>
  <span v-else class="entity-link entity-link--static" :title="staticTitle">
    <KindPill v-if="showKind" :kind="entity.kind" />
    <span class="entity-link__id">{{ entity.id }}</span>
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
</style>
