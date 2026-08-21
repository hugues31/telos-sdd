<script setup lang="ts">
// Compact status indicator for an intent. Colour comes from the same
// `--color-status-*` tokens the header uses for the project-state dot
// (src/components/AppHeader.vue) — active reads as "ok", draft as
// "in progress", deprecated as "no longer valid".
import { computed } from 'vue';

import type { IntentStatus } from '../data/types';

const props = defineProps<{
  status: IntentStatus;
}>();

const labels: Record<IntentStatus, string> = {
  active: 'Active',
  draft: 'Draft',
  deprecated: 'Deprecated',
};

const label = computed(() => labels[props.status]);
</script>

<template>
  <span class="status-badge" :class="`status-badge--${status}`">{{ label }}</span>
</template>

<style scoped>
.status-badge {
  display: inline-flex;
  align-items: center;
  padding: 0.0625rem 0.5rem;
  border: 1px solid currentColor;
  border-radius: 999px;
  font-size: 0.75rem;
  font-weight: 600;
  line-height: 1.4;
  white-space: nowrap;
}

.status-badge--active {
  color: var(--color-status-ok);
}

.status-badge--draft {
  color: var(--color-status-warn);
}

.status-badge--deprecated {
  color: var(--color-status-error);
}
</style>
