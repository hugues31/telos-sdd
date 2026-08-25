<script setup lang="ts">
// Intent list: filtered by free-text search + status, both synced into
// `route.query` (`?q=...&status=...`) rather than local component state, so
// the URL alone is always enough to reproduce a given view — a reload or a
// shared link restores the same filter. `router.replace` (never `push`)
// keeps every keystroke from growing browser history.
import { computed } from 'vue';
import { useRoute, useRouter } from 'vue-router';

import EmptyState from '../components/EmptyState.vue';
import SearchInput from '../components/SearchInput.vue';
import StatusBadge from '../components/StatusBadge.vue';
import { snapshot } from '../data/snapshot';
import type { IntentStatus, IntentView } from '../data/types';

const route = useRoute();
const router = useRouter();

const intents = computed(() => snapshot.value.snapshot.intents);

const statuses: IntentStatus[] = ['active', 'draft', 'deprecated'];

type QueryValue = string | null | (string | null)[] | undefined;

function firstQueryValue(raw: QueryValue): string {
  const value = Array.isArray(raw) ? raw[0] : raw;
  return value ?? '';
}

const searchQuery = computed(() => firstQueryValue(route.query.q));

const statusQuery = computed<IntentStatus | null>(() => {
  const value = firstQueryValue(route.query.status);
  return (statuses as string[]).includes(value) ? (value as IntentStatus) : null;
});
const ownerQuery = computed(() => firstQueryValue(route.query.owner));
const owners = computed(() => [...new Set(intents.value.map((intent) => intent.owner))].sort());

function setSearchQuery(value: string): void {
  router.replace({ query: { ...route.query, q: value === '' ? undefined : value } });
}

function setStatusQuery(value: IntentStatus | null): void {
  router.replace({ query: { ...route.query, status: value ?? undefined } });
}

function setOwnerQuery(value: string): void {
  router.replace({ query: { ...route.query, owner: value === '' ? undefined : value } });
}

const filteredIntents = computed<IntentView[]>(() => {
  const q = searchQuery.value.trim().toLowerCase();
  return intents.value.filter((intent) => {
    if (statusQuery.value && intent.status !== statusQuery.value) return false;
    if (ownerQuery.value && intent.owner !== ownerQuery.value) return false;
    if (!q) return true;
    return (
      intent.id.toLowerCase().includes(q) ||
      intent.title.toLowerCase().includes(q) ||
      intent.telos.toLowerCase().includes(q)
      || intent.owner.toLowerCase().includes(q)
    );
  });
});

const isFiltered = computed(
  () => searchQuery.value.trim() !== '' || statusQuery.value !== null || ownerQuery.value !== '',
);

function plural(count: number, word: string): string {
  return `${count} ${word}${count === 1 ? '' : 's'}`;
}
</script>

<template>
  <section class="page intents-page">
    <h1>Intents</h1>

    <div class="intents-page__filters">
      <SearchInput
        :model-value="searchQuery"
        ariaLabel="Search intents"
        placeholder="Search by id, title or telos…"
        @update:model-value="setSearchQuery"
      />

      <div class="status-filter" role="group" aria-label="Filter by status">
        <button
          type="button"
          class="status-filter__option"
          :class="{ 'status-filter__option--active': statusQuery === null }"
          :aria-pressed="statusQuery === null"
          @click="setStatusQuery(null)"
        >
          All
        </button>
        <button
          v-for="status in statuses"
          :key="status"
          type="button"
          class="status-filter__option"
          :class="{ 'status-filter__option--active': statusQuery === status }"
          :aria-pressed="statusQuery === status"
          @click="setStatusQuery(status)"
        >
          <StatusBadge :status="status" />
        </button>
      </div>
      <label class="owner-filter">
        <span class="sr-only">Filter by domain owner</span>
        <select :value="ownerQuery" @change="setOwnerQuery(($event.target as HTMLSelectElement).value)">
          <option value="">All contexts and capabilities</option>
          <option v-for="owner in owners" :key="owner" :value="owner">{{ owner }}</option>
        </select>
      </label>
    </div>

    <p class="intents-page__count">
      <template v-if="isFiltered">{{ filteredIntents.length }} of {{ intents.length }} intents</template>
      <template v-else>{{ plural(intents.length, 'intent') }}</template>
    </p>

    <ul v-if="filteredIntents.length" class="intent-list">
      <li v-for="intent in filteredIntents" :key="intent.id">
        <RouterLink :to="{ name: 'intent-detail', params: { id: intent.id } }" class="intent-row">
          <div class="intent-row__heading">
            <span class="intent-row__id">{{ intent.id }}</span>
            <span class="intent-row__title">{{ intent.title }}</span>
            <StatusBadge :status="intent.status" />
          </div>
          <div class="intent-row__meta">
            <span>{{ intent.owner }}</span>
            <span>{{ plural(intent.scenarios.length, 'scenario') }}</span>
            <span>{{ plural(intent.notions.length, 'notion') }}</span>
          </div>
        </RouterLink>
      </li>
    </ul>

    <EmptyState
      v-else-if="intents.length === 0"
      title="No intents yet"
      text="Declare intents in your .tel files to see them here."
    />
    <EmptyState
      v-else
      title="No intents match your search"
      text="Try a different search term or status filter."
    />
  </section>
</template>

<style scoped>
.intents-page__filters {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 1rem;
  margin-bottom: 1rem;
}

.intents-page__filters .search-input {
  flex: 1 1 16rem;
  max-width: 24rem;
}

.status-filter {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
}

.owner-filter select { padding: 0.5rem 0.7rem; border: 1px solid var(--color-border); border-radius: 0.45rem; color: var(--color-text); background: var(--color-surface); }

.status-filter__option {
  display: inline-flex;
  align-items: center;
  border: 1px solid var(--color-border);
  border-radius: 999px;
  background: var(--color-surface);
  padding: 0.125rem 0.625rem;
  color: var(--color-text-muted);
  cursor: pointer;
  font-size: 0.8125rem;
}

.status-filter__option:hover {
  border-color: var(--color-primary);
}

.status-filter__option--active {
  border-color: var(--color-primary);
  background: var(--color-primary-soft);
  color: var(--color-text);
}

.intents-page__count {
  color: var(--color-text-muted);
  font-size: 0.875rem;
  margin-bottom: 1rem;
}

.intent-list {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  list-style: none;
  margin: 0;
  padding: 0;
}

.intent-row {
  display: block;
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: 0.75rem;
  padding: 1rem 1.25rem;
  color: var(--color-text);
}

.intent-row:hover {
  border-color: var(--color-primary);
  text-decoration: none;
}

.intent-row__heading {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.625rem;
}

.intent-row__id {
  font-family:
    ui-monospace,
    SFMono-Regular,
    Menlo,
    Consolas,
    'Liberation Mono',
    monospace;
  font-size: 0.8125rem;
  color: var(--color-text-muted);
}

.intent-row__title {
  font-weight: 600;
  flex: 1 1 auto;
}

.intent-row__meta {
  display: flex;
  gap: 1rem;
  margin-top: 0.375rem;
  font-size: 0.8125rem;
  color: var(--color-text-muted);
}
</style>
