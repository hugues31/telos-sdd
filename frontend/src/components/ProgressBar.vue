<script setup lang="ts">
// Accessible 0-100 progress meter. `color` names a CSS custom property
// (e.g. "--color-status-ok"), never a literal colour, so every fill still
// resolves through tokens.css.
import { computed } from 'vue';

const props = withDefaults(
  defineProps<{
    /** 0-100; out-of-range values are clamped. */
    value: number;
    /** Name of a CSS custom property, e.g. "--color-primary". */
    color?: string;
    /** aria-label for the meter; omit when a visible label already describes it. */
    label?: string;
  }>(),
  { color: '--color-primary', label: undefined },
);

const clamped = computed(() => Math.min(100, Math.max(0, Math.round(props.value))));
</script>

<template>
  <div
    class="progress-bar"
    role="progressbar"
    :aria-valuenow="clamped"
    aria-valuemin="0"
    aria-valuemax="100"
    :aria-label="label"
  >
    <div class="progress-bar__fill" :style="{ width: `${clamped}%`, background: `var(${color})` }"></div>
  </div>
</template>

<style scoped>
.progress-bar {
  width: 100%;
  height: 0.5rem;
  border-radius: 999px;
  background: var(--color-bg-subtle);
  overflow: hidden;
}

.progress-bar__fill {
  height: 100%;
  border-radius: inherit;
  transition: width 0.2s ease;
}
</style>
