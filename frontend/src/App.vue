<script setup lang="ts">
import AppFooter from './components/AppFooter.vue';
import AppHeader from './components/AppHeader.vue';
</script>

<template>
  <AppHeader />
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
</style>
