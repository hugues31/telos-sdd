<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';

import type { GraphNodeView } from '../data/types';
import { searchEntities } from '../search/entities';
import KindPill from './KindPill.vue';

const props = defineProps<{ nodes: GraphNodeView[] }>();
const emit = defineEmits<{ choose: [node: GraphNodeView] }>();

const root = ref<HTMLElement | null>(null);
const query = ref('');
const activeIndex = ref(0);
const listOpen = ref(false);
const results = computed(() => searchEntities(props.nodes, query.value));
const activeResult = computed(() => results.value[activeIndex.value]);

watch(query, () => {
  activeIndex.value = 0;
  listOpen.value = query.value.trim().length > 0;
});

function choose(node: GraphNodeView): void {
  emit('choose', node);
  listOpen.value = false;
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault();
    listOpen.value = false;
    return;
  }
  if (results.value.length === 0) return;

  if (event.key === 'ArrowDown') {
    event.preventDefault();
    listOpen.value = true;
    activeIndex.value = (activeIndex.value + 1) % results.value.length;
    return;
  }
  if (event.key === 'ArrowUp') {
    event.preventDefault();
    listOpen.value = true;
    activeIndex.value = (activeIndex.value - 1 + results.value.length) % results.value.length;
    return;
  }
  if (event.key === 'Enter' && listOpen.value && activeResult.value) {
    event.preventDefault();
    choose(activeResult.value);
  }
}

function onDocumentPointerdown(event: PointerEvent): void {
  if (!root.value?.contains(event.target as Node)) listOpen.value = false;
}

onMounted(() => document.addEventListener('pointerdown', onDocumentPointerdown));
onUnmounted(() => document.removeEventListener('pointerdown', onDocumentPointerdown));
</script>

<template>
  <div ref="root" class="graph-finder">
    <label for="graph-node-finder">Find node</label>
    <div class="graph-finder__control">
      <span aria-hidden="true">⌕</span>
      <input
        id="graph-node-finder"
        v-model="query"
        type="search"
        autocomplete="off"
        placeholder="Name, ID, or type"
        role="combobox"
        aria-autocomplete="list"
        aria-controls="graph-finder-results"
        :aria-expanded="listOpen"
        :aria-activedescendant="
          listOpen && activeResult ? `graph-finder-result-${activeIndex}` : undefined
        "
        @focus="listOpen = query.trim().length > 0"
        @keydown="onKeydown"
      />
    </div>

    <div v-if="listOpen" class="graph-finder__popover">
      <ul
        v-if="results.length"
        id="graph-finder-results"
        class="graph-finder__results"
        role="listbox"
        aria-label="Matching graph nodes"
      >
        <li v-for="(node, index) in results" :key="`${node.key.kind}:${node.key.id}`">
          <button
            :id="`graph-finder-result-${index}`"
            type="button"
            role="option"
            :aria-selected="index === activeIndex"
            :class="{ 'graph-finder__result--active': index === activeIndex }"
            @mouseenter="activeIndex = index"
            @click="choose(node)"
          >
            <KindPill :kind="node.key.kind" />
            <span class="graph-finder__copy">
              <strong>{{ node.label }}</strong>
              <span>{{ node.key.id }}</span>
            </span>
          </button>
        </li>
      </ul>
      <p v-else class="graph-finder__empty">No matching nodes</p>
    </div>
  </div>
</template>

<style scoped>
.graph-finder {
  position: relative;
  min-width: min(18rem, 100%);
  color: var(--color-text-muted);
  font-size: 0.875rem;
  font-weight: 600;
}

.graph-finder > label {
  display: block;
  margin-bottom: 0.25rem;
}

.graph-finder__control {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  min-height: 2.5rem;
  padding-inline: 0.625rem;
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: 0.375rem;
}

.graph-finder__control:focus-within {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px var(--color-primary-soft);
}

.graph-finder__control > span {
  color: var(--color-primary-strong);
  font-size: 1.125rem;
}

.graph-finder input {
  width: 100%;
  min-width: 0;
  padding: 0;
  color: var(--color-text);
  background: transparent;
  border: 0;
  outline: 0;
  font: inherit;
  font-weight: 400;
}

.graph-finder__popover {
  position: absolute;
  z-index: 5;
  top: calc(100% + 0.375rem);
  right: 0;
  left: 0;
  overflow: hidden;
  background: var(--color-surface-raised);
  border: 1px solid var(--color-border);
  border-radius: 0.5rem;
  box-shadow: 0 0.75rem 2rem rgb(8 24 48 / 18%);
}

.graph-finder__results {
  max-height: 20rem;
  margin: 0;
  padding: 0.375rem;
  overflow-y: auto;
  list-style: none;
}

.graph-finder__results button {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  width: 100%;
  min-height: 2.75rem;
  padding: 0.5rem;
  text-align: left;
  background: transparent;
  border: 0;
  border-radius: 0.375rem;
  cursor: pointer;
}

.graph-finder__results button:hover,
.graph-finder__result--active {
  background: var(--color-primary-soft) !important;
}

.graph-finder__copy {
  display: grid;
  min-width: 0;
}

.graph-finder__copy strong,
.graph-finder__copy span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.graph-finder__copy strong {
  color: var(--color-text);
}

.graph-finder__copy span {
  color: var(--color-text-muted);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.75rem;
  font-weight: 400;
}

.graph-finder__empty {
  margin: 0;
  padding: 0.875rem;
  font-weight: 400;
  text-align: center;
}
</style>
