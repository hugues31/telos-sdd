<script setup lang="ts">
import { computed, ref } from 'vue';

import EmptyState from '../components/EmptyState.vue';
import EntityLink from '../components/EntityLink.vue';
import KindPill from '../components/KindPill.vue';
import SearchInput from '../components/SearchInput.vue';
import TelCode from '../components/TelCode.vue';
import { notionUsedBy, snapshot } from '../data/snapshot';
import { filterNotions, groupNotions } from './page-data';

const query = ref('');
const owner = ref('');
const owners = computed(() => [...new Set(snapshot.value.snapshot.notions.map((notion) => notion.owner))].sort());
const filteredNotions = computed(() =>
  filterNotions(snapshot.value.snapshot.notions, query.value).filter(
    (notion) => owner.value === '' || notion.owner === owner.value,
  ),
);
const groups = computed(() => groupNotions(filteredNotions.value));
</script>

<template>
  <section class="page glossary">
    <h1>Glossary</h1>

    <div class="glossary__layout">
      <aside class="glossary__toc" aria-labelledby="glossary-toc-heading">
        <h2 id="glossary-toc-heading">Contents</h2>
        <SearchInput v-model="query" ariaLabel="Search glossary" placeholder="Search notions" />
        <label class="glossary__owner-filter">
          Domain owner
          <select v-model="owner">
            <option value="">All contexts and capabilities</option>
            <option v-for="item in owners" :key="item" :value="item">{{ item }}</option>
          </select>
        </label>
        <p class="glossary__count" role="status">
          {{ filteredNotions.length }} {{ filteredNotions.length === 1 ? 'notion' : 'notions' }}
        </p>

        <nav v-if="groups.length" aria-label="Glossary entries">
          <section v-for="group in groups" :key="`${group.owner}-${group.kind}`" class="glossary__toc-group">
            <h3>{{ group.owner }} · {{ group.kind }}</h3>
            <ul>
              <li v-for="notion in group.notions" :key="notion.name">
                <RouterLink :to="{ hash: `#notion-${notion.name}` }">{{ notion.name }}</RouterLink>
              </li>
            </ul>
          </section>
        </nav>
      </aside>

      <div class="glossary__content" aria-live="polite">
        <EmptyState
          v-if="!filteredNotions.length"
          title="No matching notions"
          text="Try a different search term."
        />

        <section
          v-else
          v-for="group in groups"
          :key="`${group.owner}-${group.kind}`"
          class="glossary__group"
          :aria-labelledby="`notion-kind-${group.owner}-${group.kind}`"
        >
          <h2 :id="`notion-kind-${group.owner}-${group.kind}`">{{ group.owner }} · {{ group.kind }}</h2>
          <article
            v-for="notion in group.notions"
            :id="`notion-${notion.name}`"
            :key="notion.name"
            class="glossary-card"
          >
            <header class="glossary-card__header">
              <h3>{{ notion.name }}</h3>
              <KindPill :kind="notion.kind" />
            </header>
            <p>{{ notion.definition }}</p>

            <div class="glossary-card__consumers">
              <h4>Used by</h4>
              <ul v-if="notionUsedBy.get(notion.name)?.length" class="glossary-card__consumer-list">
                <li v-for="consumer in notionUsedBy.get(notion.name)" :key="`${consumer.kind}-${consumer.id}`">
                  <EntityLink :entity="consumer" :show-kind="false" />
                </li>
              </ul>
              <p v-else class="glossary-card__muted">No consumers recorded.</p>
            </div>

            <details class="glossary-card__source">
              <summary>Canonical .tel source</summary>
              <TelCode :source="notion.canonical" />
            </details>
          </article>
        </section>
      </div>
    </div>
  </section>
</template>

<style scoped>
.glossary__layout { display: grid; grid-template-columns: 260px minmax(0, 1fr); gap: 2rem; }
.glossary__toc { align-self: start; position: sticky; top: calc(var(--header-height) + 1rem); max-height: calc(100vh - var(--header-height) - 2rem); overflow-y: auto; padding-right: 0.5rem; }
.glossary__toc h2, .glossary__group > h2 { font-size: 1.125rem; }
.glossary__count, .glossary-card__muted { color: var(--color-text-muted); font-size: 0.875rem; }
.glossary__owner-filter { display: grid; gap: 0.35rem; margin-top: 0.75rem; color: var(--color-text-muted); font-size: 0.8125rem; }
.glossary__owner-filter select { width: 100%; padding: 0.5rem; border: 1px solid var(--color-border); border-radius: 0.4rem; color: var(--color-text); background: var(--color-surface); }
.glossary__toc-group { margin-top: 1rem; }
.glossary__toc-group h3 { margin-bottom: 0.25rem; color: var(--color-text-muted); font-size: 0.8125rem; text-transform: capitalize; }
.glossary__toc-group ul, .glossary-card__consumer-list { margin: 0; padding: 0; list-style: none; }
.glossary__toc-group li + li { margin-top: 0.25rem; }
.glossary-card__consumer-list { display: flex; flex-wrap: wrap; gap: 0.375rem 0.75rem; }
.glossary__group + .glossary__group { margin-top: 2rem; }
.glossary-card { scroll-margin-top: calc(var(--header-height) + 1rem); background: var(--color-surface); border: 1px solid var(--color-border); border-radius: 0.75rem; padding: 1.25rem; }
.glossary-card + .glossary-card { margin-top: 1rem; }
.glossary-card__header { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 0.75rem; }
.glossary-card__header h3, .glossary-card__consumers h4 { margin: 0; }
.glossary-card__consumers { margin-top: 1rem; }
.glossary-card__consumers h4 { margin-bottom: 0.375rem; font-size: 0.875rem; }
.glossary-card__source { margin-top: 1rem; }
.glossary-card__source summary { cursor: pointer; color: var(--color-link); font-weight: 600; }
@media (max-width: 720px) { .glossary__layout { grid-template-columns: minmax(0, 1fr); } .glossary__toc { position: static; max-height: none; overflow: visible; padding-right: 0; } }
</style>
