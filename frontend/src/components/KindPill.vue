<script setup lang="ts">
// Pastille identifying a graph entity kind. Colours come from the `--k-*`
// tokens (src/styles/tokens.css) — the same ones the future Cytoscape graph
// reads via getComputedStyle, so a kind always looks the same everywhere in
// the app. The dot alone carries the colour; the label always accompanies
// it so kind is never conveyed by colour alone.
import { computed } from 'vue';

import type { GraphKeyKind } from '../data/types';

const props = withDefaults(
  defineProps<{
    kind: GraphKeyKind;
    /** Override the default kind label (e.g. a plural in a list heading). */
    label?: string;
  }>(),
  { label: undefined },
);

const defaultLabels: Record<GraphKeyKind, string> = {
  notion: 'Notion',
  intent: 'Intent',
  scenario: 'Scenario',
  constraint: 'Constraint',
  code: 'Code',
  test: 'Test',
};

const displayLabel = computed(() => props.label ?? defaultLabels[props.kind]);
</script>

<template>
  <span class="kind-pill" :class="`kind-pill--${kind}`">
    <span class="kind-pill__dot" aria-hidden="true"></span>
    <span class="kind-pill__label">{{ displayLabel }}</span>
  </span>
</template>

<style scoped>
.kind-pill {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  font-size: 0.8125rem;
  color: var(--color-text-muted);
  white-space: nowrap;
}

.kind-pill__dot {
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  flex-shrink: 0;
}

.kind-pill--notion .kind-pill__dot {
  background: var(--k-notion);
}

.kind-pill--intent .kind-pill__dot {
  background: var(--k-intent);
}

.kind-pill--scenario .kind-pill__dot {
  background: var(--k-scenario);
}

.kind-pill--constraint .kind-pill__dot {
  background: var(--k-constraint);
}

.kind-pill--code .kind-pill__dot {
  background: var(--k-code);
}

.kind-pill--test .kind-pill__dot {
  background: var(--k-test);
}
</style>
