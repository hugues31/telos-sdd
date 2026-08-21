<script setup lang="ts">
// Pastille identifying a graph entity kind. Colours come from the `--k-*`
// tokens (src/styles/tokens.css) — the same ones the future Cytoscape graph
// reads via getComputedStyle, so a kind always looks the same everywhere in
// the app. The dot alone carries the colour; the label always accompanies
// it so kind is never conveyed by colour alone.
import { computed } from 'vue';

import type { ConstraintKind, GraphKeyKind, NotionKind } from '../data/types';

type PillKind = GraphKeyKind | NotionKind | ConstraintKind;

const props = withDefaults(
  defineProps<{
    kind: PillKind;
    /** Override the default kind label (e.g. a plural in a list heading). */
    label?: string;
  }>(),
  { label: undefined },
);

const defaultLabels: Record<PillKind, string> = {
  notion: 'Notion',
  intent: 'Intent',
  scenario: 'Scenario',
  constraint: 'Constraint',
  code: 'Code',
  test: 'Test',
  actor: 'Actor',
  entity: 'Entity',
  value: 'Value',
  event: 'Event',
  state: 'State',
  stack: 'Stack',
  architecture: 'Architecture',
  quality: 'Quality',
  security: 'Security',
  convention: 'Convention',
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

.kind-pill--actor .kind-pill__dot,
.kind-pill--entity .kind-pill__dot,
.kind-pill--value .kind-pill__dot,
.kind-pill--event .kind-pill__dot,
.kind-pill--state .kind-pill__dot {
  background: var(--k-notion);
}

.kind-pill--stack .kind-pill__dot,
.kind-pill--architecture .kind-pill__dot,
.kind-pill--quality .kind-pill__dot,
.kind-pill--security .kind-pill__dot,
.kind-pill--convention .kind-pill__dot {
  background: var(--k-constraint);
}
</style>
