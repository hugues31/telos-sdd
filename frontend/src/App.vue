<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue';

import AppFooter from './components/AppFooter.vue';
import AppHeader from './components/AppHeader.vue';
import { liveState, startLiveReload } from './data/live';
import { replaceSnapshot, snapshot } from './data/snapshot';

let stopLiveReload: (() => void) | undefined;

onMounted(() => {
  stopLiveReload = startLiveReload(snapshot.value.meta.mode, replaceSnapshot);
});

onUnmounted(() => {
  stopLiveReload?.();
});
</script>

<template>
  <AppHeader />
  <aside
    v-if="
      liveState.reload_error !== null ||
      liveState.watcher_error !== null ||
      liveState.client_error !== null
    "
    class="live-alert"
    role="alert"
    aria-live="assertive"
  >
    <p v-if="liveState.reload_error !== null">
      <strong>Reload error:</strong> {{ liveState.reload_error }}
    </p>
    <p v-if="liveState.watcher_error !== null">
      <strong>Watcher error:</strong> {{ liveState.watcher_error }}
    </p>
    <p v-if="liveState.client_error !== null">
      <strong>Refresh error:</strong> {{ liveState.client_error }}
    </p>
  </aside>
  <main class="app-main">
    <RouterView />
  </main>
  <AppFooter />
</template>

<style scoped>
.app-main {
  max-width: 72rem;
  min-height: calc(100vh - var(--header-height));
  margin: 0 auto;
  padding-inline: 1.5rem;
}

.live-alert {
  max-width: 72rem;
  margin: 1rem auto 0;
  padding: 0.75rem 1.5rem;
  color: var(--color-text);
  background: var(--color-status-warn-bg);
  border: 1px solid var(--color-status-error);
  border-radius: 0.5rem;
}

.live-alert p {
  margin: 0;
}

.live-alert p + p {
  margin-top: 0.25rem;
}
</style>
