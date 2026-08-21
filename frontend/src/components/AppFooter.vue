<script setup lang="ts">
import { computed } from 'vue';

import { snapshot } from '../data/snapshot';

const meta = computed(() => snapshot.value.meta);
</script>

<template>
  <footer class="app-footer">
    <div class="app-footer__inner">telos {{ meta.version }} · built {{ meta.build_date }}</div>
  </footer>
</template>

<style scoped>
.app-footer {
  border-top: 1px solid var(--color-border);
  color: var(--color-text-muted);
  font-size: 0.875rem;
}

.app-footer__inner {
  max-width: 72rem;
  margin: 0 auto;
  padding: 1rem 1.5rem;
}
</style>
