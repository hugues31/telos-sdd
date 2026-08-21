<script setup lang="ts">
// Dashboard metric tile: a label, a big value, an optional subtext, and an
// optional slot for extra content (e.g. a ProgressBar). Passing `to` turns
// the whole card into a RouterLink — the "quick link to the relevant page"
// pattern the dashboard uses (e.g. the Intents card -> /intents).
import type { RouteLocationRaw } from 'vue-router';

withDefaults(
  defineProps<{
    label: string;
    value: string | number;
    subtext?: string;
    to?: RouteLocationRaw;
  }>(),
  { subtext: undefined, to: undefined },
);
</script>

<template>
  <RouterLink v-if="to" :to="to" class="metric-card metric-card--link">
    <div class="metric-card__label">{{ label }}</div>
    <div class="metric-card__value">{{ value }}</div>
    <div v-if="subtext" class="metric-card__subtext">{{ subtext }}</div>
    <div v-if="$slots.default" class="metric-card__extra"><slot /></div>
  </RouterLink>
  <div v-else class="metric-card">
    <div class="metric-card__label">{{ label }}</div>
    <div class="metric-card__value">{{ value }}</div>
    <div v-if="subtext" class="metric-card__subtext">{{ subtext }}</div>
    <div v-if="$slots.default" class="metric-card__extra"><slot /></div>
  </div>
</template>

<style scoped>
.metric-card {
  display: block;
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: 0.75rem;
  padding: 1rem 1.25rem;
  color: var(--color-text);
}

.metric-card--link:hover {
  border-color: var(--color-primary);
  text-decoration: none;
}

.metric-card__label {
  font-size: 0.8125rem;
  color: var(--color-text-muted);
  margin-bottom: 0.25rem;
}

.metric-card__value {
  font-size: 1.75rem;
  font-weight: 700;
  line-height: 1.2;
}

.metric-card__subtext {
  font-size: 0.8125rem;
  color: var(--color-text-muted);
  margin-top: 0.25rem;
}

.metric-card__extra {
  margin-top: 0.75rem;
}
</style>
