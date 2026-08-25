<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { useRouter } from 'vue-router';

import { scenarioToIntent, snapshot } from '../data/snapshot';
import type { GraphNodeView } from '../data/types';
import { entityDestination } from '../search/destinations';
import { searchEntities, shouldOpenGlobalSearch } from '../search/entities';
import KindPill from './KindPill.vue';

const router = useRouter();
const dialog = ref<HTMLDialogElement | null>(null);
const input = ref<HTMLInputElement | null>(null);
const query = ref('');
const activeIndex = ref(0);
const isOpen = ref(false);

const results = computed(() => searchEntities(snapshot.value.snapshot.nodes, query.value));
const activeResult = computed(() => results.value[activeIndex.value]);

watch(results, () => {
  activeIndex.value = 0;
});

async function open(): Promise<void> {
  query.value = '';
  activeIndex.value = 0;
  if (!dialog.value?.open) dialog.value?.showModal();
  isOpen.value = true;
  await nextTick();
  input.value?.focus();
}

function close(): void {
  if (dialog.value?.open) dialog.value.close();
  isOpen.value = false;
}

async function choose(node: GraphNodeView): Promise<void> {
  const parent =
    node.key.kind === 'scenario' ? scenarioToIntent.value.get(node.key.id) : undefined;
  const destination = entityDestination(node.key, parent);
  if (destination) await router.push(destination);
  close();
}

function onWindowKeydown(event: KeyboardEvent): void {
  if (!shouldOpenGlobalSearch(event)) return;
  event.preventDefault();
  void open();
}

function onInputKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault();
    close();
    return;
  }

  if (results.value.length === 0) return;
  if (event.key === 'ArrowDown') {
    event.preventDefault();
    activeIndex.value = (activeIndex.value + 1) % results.value.length;
    return;
  }
  if (event.key === 'ArrowUp') {
    event.preventDefault();
    activeIndex.value = (activeIndex.value - 1 + results.value.length) % results.value.length;
    return;
  }
  if (event.key === 'Enter' && activeResult.value) {
    event.preventDefault();
    void choose(activeResult.value);
  }
}

function onBackdropClick(event: MouseEvent): void {
  if (event.target === dialog.value) close();
}

onMounted(() => window.addEventListener('keydown', onWindowKeydown));
onUnmounted(() => window.removeEventListener('keydown', onWindowKeydown));

defineExpose({ open });
</script>

<template>
  <dialog
    ref="dialog"
    class="global-search"
    aria-labelledby="global-search-title"
    @click="onBackdropClick"
    @close="isOpen = false"
  >
    <div class="global-search__panel">
      <div class="global-search__heading">
        <div>
          <h2 id="global-search-title">Search Telos</h2>
          <p>Jump to any context, intent, scenario, notion, constraint, source, or test.</p>
        </div>
        <button type="button" class="global-search__close" aria-label="Close search" @click="close">
          <span aria-hidden="true">×</span>
        </button>
      </div>

      <label class="global-search__field">
        <span class="sr-only">Search all Telos entities</span>
        <span class="global-search__icon" aria-hidden="true">⌕</span>
        <input
          ref="input"
          v-model="query"
          type="search"
          autocomplete="off"
          placeholder="Search by name, ID, or type…"
          role="combobox"
          aria-autocomplete="list"
          aria-controls="global-search-results"
          :aria-expanded="isOpen"
          :aria-activedescendant="activeResult ? `global-search-result-${activeIndex}` : undefined"
          @keydown="onInputKeydown"
        />
      </label>

      <ul
        v-if="results.length"
        id="global-search-results"
        class="global-search__results"
        role="listbox"
        aria-label="Search results"
      >
        <li v-for="(node, index) in results" :key="`${node.key.kind}:${node.key.id}`">
          <button
            :id="`global-search-result-${index}`"
            type="button"
            role="option"
            class="global-search__result"
            :class="{ 'global-search__result--active': index === activeIndex }"
            :aria-selected="index === activeIndex"
            @mouseenter="activeIndex = index"
            @click="choose(node)"
          >
            <KindPill :kind="node.key.kind" />
            <span class="global-search__result-copy">
              <strong>{{ node.label }}</strong>
              <span>{{ node.key.id }}</span>
            </span>
            <span aria-hidden="true">↵</span>
          </button>
        </li>
      </ul>
      <p v-else-if="query.trim()" class="global-search__empty">No matching entities</p>
      <p v-else class="global-search__hint">Start typing to search the current snapshot.</p>
    </div>
  </dialog>
</template>

<style scoped>
.global-search {
  width: min(42rem, calc(100vw - 2rem));
  max-height: min(38rem, calc(100vh - 2rem));
  padding: 0;
  color: var(--color-text);
  background: transparent;
  border: 0;
  overflow: visible;
}

.global-search::backdrop {
  background: rgb(8 22 43 / 62%);
  backdrop-filter: blur(3px);
}

.global-search__panel {
  overflow: hidden;
  background: var(--color-surface-raised);
  border: 1px solid var(--color-border);
  border-radius: 0.875rem;
  box-shadow: 0 1.25rem 4rem rgb(8 24 48 / 28%);
}

.global-search__heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 1rem;
  padding: 1.25rem 1.25rem 0.875rem;
}

.global-search__heading h2 {
  margin-bottom: 0.125rem;
  font-size: 1.125rem;
}

.global-search__heading p {
  margin: 0;
  color: var(--color-text-muted);
  font-size: 0.875rem;
}

.global-search__close {
  display: inline-grid;
  width: 2.5rem;
  height: 2.5rem;
  padding: 0;
  place-items: center;
  flex: 0 0 auto;
  color: var(--color-text-muted);
  background: transparent;
  border: 0;
  border-radius: 0.5rem;
  cursor: pointer;
  font-size: 1.5rem;
  line-height: 1;
}

.global-search__close:hover {
  color: var(--color-text);
  background: var(--color-bg-subtle);
}

.global-search__field {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  margin: 0 1.25rem 0.75rem;
  padding-inline: 0.875rem;
  background: var(--color-bg-subtle);
  border: 1px solid var(--color-border);
  border-radius: 0.625rem;
}

.global-search__field:focus-within {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px var(--color-primary-soft);
}

.global-search__icon {
  color: var(--color-primary-strong);
  font-size: 1.4rem;
}

.global-search__field input {
  width: 100%;
  min-height: 3rem;
  padding: 0;
  color: var(--color-text);
  background: transparent;
  border: 0;
  outline: 0;
  font: inherit;
}

.global-search__field input::placeholder {
  color: var(--color-text-muted);
}

.global-search__results {
  max-height: min(24rem, 52vh);
  margin: 0;
  padding: 0.25rem 0.75rem 0.75rem;
  overflow-y: auto;
  list-style: none;
}

.global-search__result {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  width: 100%;
  min-height: 3.25rem;
  padding: 0.625rem 0.75rem;
  text-align: left;
  background: transparent;
  border: 0;
  border-radius: 0.5rem;
  cursor: pointer;
}

.global-search__result:hover,
.global-search__result--active {
  background: var(--color-primary-soft);
}

.global-search__result-copy {
  display: grid;
  min-width: 0;
  flex: 1;
}

.global-search__result-copy strong,
.global-search__result-copy span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.global-search__result-copy span {
  color: var(--color-text-muted);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.75rem;
}

.global-search__empty,
.global-search__hint {
  min-height: 5rem;
  margin: 0;
  padding: 1.25rem;
  color: var(--color-text-muted);
  text-align: center;
}

@media (max-width: 36rem) {
  .global-search__heading p {
    display: none;
  }
}
</style>
