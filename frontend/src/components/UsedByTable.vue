<script setup lang="ts">
import { computed, ref } from 'vue';

import { intentById, scenarioById } from '../data/snapshot';
import type { GraphKey } from '../data/types';
import {
  consumerRows,
  filterConsumerRows,
  sortConsumerRows,
  type ConsumerKind,
  type ConsumerSort,
  type SortDirection,
} from '../pages/glossary-consumers';
import EntityLink from './EntityLink.vue';
import KindPill from './KindPill.vue';

const props = defineProps<{
  consumers: GraphKey[];
  kindFilter: '' | ConsumerKind;
}>();

const sort = ref<ConsumerSort>('kind');
const direction = ref<SortDirection>('asc');
const rows = computed(() =>
  consumerRows(props.consumers, intentById.value, scenarioById.value),
);
const visibleRows = computed(() =>
  sortConsumerRows(filterConsumerRows(rows.value, props.kindFilter), sort.value, direction.value),
);

function toggleSort(nextSort: ConsumerSort): void {
  if (sort.value === nextSort) {
    direction.value = direction.value === 'asc' ? 'desc' : 'asc';
    return;
  }
  sort.value = nextSort;
  direction.value = 'asc';
}

function ariaSort(column: ConsumerSort): 'ascending' | 'descending' | 'none' {
  if (sort.value !== column) return 'none';
  return direction.value === 'asc' ? 'ascending' : 'descending';
}
</script>

<template>
  <div v-if="visibleRows.length" class="used-by-table" tabindex="0" aria-label="Used by entities">
    <table>
      <thead>
        <tr>
          <th scope="col" :aria-sort="ariaSort('kind')">
            <button type="button" @click="toggleSort('kind')">
              Type <span aria-hidden="true">{{ sort === 'kind' ? (direction === 'asc' ? '↑' : '↓') : '↕' }}</span>
            </button>
          </th>
          <th scope="col" :aria-sort="ariaSort('id')">
            <button type="button" @click="toggleSort('id')">
              Reference <span aria-hidden="true">{{ sort === 'id' ? (direction === 'asc' ? '↑' : '↓') : '↕' }}</span>
            </button>
          </th>
          <th scope="col" :aria-sort="ariaSort('title')">
            <button type="button" @click="toggleSort('title')">
              Title <span aria-hidden="true">{{ sort === 'title' ? (direction === 'asc' ? '↑' : '↓') : '↕' }}</span>
            </button>
          </th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in visibleRows" :key="`${row.kind}:${row.id}`">
          <td><KindPill :kind="row.kind" /></td>
          <td class="used-by-table__id">{{ row.id }}</td>
          <td><EntityLink :entity="row.entity" :show-kind="false" /></td>
        </tr>
      </tbody>
    </table>
  </div>
  <p v-else class="used-by-table__empty">No consumers recorded.</p>
</template>

<style scoped>
.used-by-table {
  overflow-x: auto;
  border: 1px solid var(--color-border);
  border-radius: 0.5rem;
}

.used-by-table:focus-visible {
  outline-offset: 2px;
}

.used-by-table table {
  min-width: 34rem;
  background: var(--color-surface);
  font-size: 0.875rem;
}

.used-by-table th {
  padding: 0;
  background: var(--color-bg-subtle);
}

.used-by-table th button {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  width: 100%;
  min-height: 2.5rem;
  padding: 0.5rem 0.75rem;
  text-align: left;
  background: transparent;
  border: 0;
  cursor: pointer;
  font-size: 0.75rem;
  font-weight: 700;
}

.used-by-table th button:hover {
  color: var(--color-primary-strong);
  background: var(--color-primary-soft);
}

.used-by-table tbody tr:last-child td {
  border-bottom: 0;
}

.used-by-table__id {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.8125rem;
  white-space: nowrap;
}

.used-by-table__empty {
  margin: 0;
  color: var(--color-text-muted);
  font-size: 0.875rem;
}
</style>
