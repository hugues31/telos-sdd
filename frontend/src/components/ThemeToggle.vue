<script setup lang="ts">
import { computed } from 'vue';

import { useTheme } from '../composables/useTheme';

const { theme, toggleTheme } = useTheme();

const label = computed(() =>
  theme.value === 'dark' ? 'Switch to light theme' : 'Switch to dark theme',
);
</script>

<template>
  <button type="button" class="theme-toggle" :aria-label="label" :title="label" @click="toggleTheme">
    <span aria-hidden="true">{{ theme === 'dark' ? '🌙' : '☀️' }}</span>
  </button>
</template>

<style scoped>
.theme-toggle {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 2.25rem;
  height: 2.25rem;
  border: 1px solid var(--color-border);
  border-radius: 0.5rem;
  background: var(--color-surface);
  cursor: pointer;
  line-height: 1;
}

.theme-toggle:hover {
  background: var(--color-bg-subtle);
}
</style>
