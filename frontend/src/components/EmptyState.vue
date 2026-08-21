<script setup lang="ts">
// Generic empty state: an icon, a title, optional supporting text, and an
// optional slot for actions. Deliberately reuses the `.empty-state` box
// (border, padding, muted text) already defined in src/styles/base.css for
// the page stubs and the no-data fallback in main.ts — this component only
// adds the icon/title/text layout inside it.
withDefaults(
  defineProps<{
    title: string;
    text?: string;
  }>(),
  { text: undefined },
);
</script>

<template>
  <div class="empty-state empty-state--component">
    <svg class="empty-state__icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <rect x="3.5" y="7" width="17" height="12" rx="1.5" stroke="currentColor" stroke-width="1.5" fill="none" />
      <path d="M3.5 11h17" stroke="currentColor" stroke-width="1.5" />
      <path d="M8 4.5h8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
    </svg>
    <p class="empty-state__title">{{ title }}</p>
    <p v-if="text" class="empty-state__text">{{ text }}</p>
    <div v-if="$slots.default" class="empty-state__actions"><slot /></div>
  </div>
</template>

<style scoped>
.empty-state--component {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
  text-align: center;
}

.empty-state__icon {
  width: 2rem;
  height: 2rem;
  color: var(--color-text-muted);
}

.empty-state__title {
  font-weight: 600;
  color: var(--color-text);
  margin: 0;
}

.empty-state__text {
  color: var(--color-text-muted);
  margin: 0;
}

.empty-state__actions {
  margin-top: 0.5rem;
}
</style>
