<script setup lang="ts">
import { computed } from 'vue';

import { snapshot } from '../data/snapshot';
import ThemeToggle from './ThemeToggle.vue';

// Top-level, browsable destinations. `/intent/:id` (IntentDetailPage) is
// deliberately not listed here: it needs a concrete intent id, so it is only
// reachable by following a link from IntentsPage (or later, the graph), never
// from the nav itself.
const links = [
  { to: '/', label: 'Dashboard' },
  { to: '/intents', label: 'Intents' },
  { to: '/graph', label: 'Graph' },
  { to: '/glossary', label: 'Glossary' },
  { to: '/coverage', label: 'Coverage' },
];

// Simple project-health heuristic: green when the snapshot's project state
// carries no error/inconsistency (`dashboard.state === 'coherent'`, i.e. no
// drift and no open change blocking it), orange for everything else
// ("changing" mid-edit, or "drifted" with unresolved drift). See
// `DashboardView.state` / `state_kind` in crates/telos/src/view/model.rs.
const projectState = computed(() => snapshot.value.snapshot.dashboard.state);
const isHealthy = computed(() => projectState.value === 'coherent');
</script>

<template>
  <header class="app-header">
    <div class="app-header__inner">
      <RouterLink to="/" class="app-header__brand">
        <span class="app-header__logo">Telos</span>
        <span
          class="app-header__status"
          :class="{ 'app-header__status--warn': !isHealthy }"
          :title="`Project state: ${projectState}`"
          :aria-label="`Project state: ${projectState}`"
          role="img"
        ></span>
      </RouterLink>

      <nav class="app-header__nav" aria-label="Main">
        <RouterLink v-for="link in links" :key="link.to" :to="link.to" class="app-header__link">
          {{ link.label }}
        </RouterLink>
      </nav>

      <ThemeToggle />
    </div>
  </header>
</template>

<style scoped>
.app-header {
  position: sticky;
  top: 0;
  z-index: 10;
  height: var(--header-height);
  background: var(--color-surface);
  border-bottom: 1px solid var(--color-border);
}

.app-header__inner {
  max-width: 72rem;
  height: 100%;
  margin: 0 auto;
  padding-inline: 1.5rem;
  display: flex;
  align-items: center;
  gap: 1.5rem;
}

.app-header__brand {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  color: var(--color-text);
}

.app-header__brand:hover {
  text-decoration: none;
}

.app-header__logo {
  font-weight: 700;
  font-size: 1.125rem;
  color: var(--color-primary-strong);
}

.app-header__status {
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  background: var(--color-status-ok);
}

.app-header__status--warn {
  background: var(--color-status-warn);
}

.app-header__nav {
  display: flex;
  gap: 1rem;
  flex: 1;
}

.app-header__link {
  color: var(--color-text-muted);
  padding-block: 0.25rem;
}

.app-header__link:hover {
  color: var(--color-text);
  text-decoration: none;
}

.app-header__link.router-link-active {
  color: var(--color-primary-strong);
  font-weight: 600;
}
</style>
