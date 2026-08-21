<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import type { RouteLocationRaw } from 'vue-router';

import EmptyState from '../components/EmptyState.vue';
import EntityLink from '../components/EntityLink.vue';
import KindPill from '../components/KindPill.vue';
import { scenarioToIntent, snapshot } from '../data/snapshot';
import type { GraphKey, GraphRelation } from '../data/types';
import CytoGraph from '../graph/CytoGraph.vue';
import type { GraphSelection, RelationFilter } from '../graph/elements';

const nodes = computed(() => snapshot.value.snapshot.nodes);
const edges = computed(() => snapshot.value.snapshot.edges);
const selected = ref<GraphSelection | null>(null);
const relationFilter = ref<RelationFilter>('all');

const relations = computed<GraphRelation[]>(() => {
  return [...new Set(edges.value.map((edge) => edge.relation))].sort();
});

watch(relations, (available) => {
  if (relationFilter.value !== 'all' && !available.includes(relationFilter.value)) {
    relationFilter.value = 'all';
  }
});

const matchingEdges = computed(() => {
  if (relationFilter.value === 'all') return edges.value;
  return edges.value.filter((edge) => edge.relation === relationFilter.value);
});

const connectedNodeCount = computed(() => {
  if (relationFilter.value === 'all') return nodes.value.length;
  const ids = new Set<string>();
  for (const edge of matchingEdges.value) {
    ids.add(`${edge.from.kind}:${edge.from.id}`);
    ids.add(`${edge.to.kind}:${edge.to.id}`);
  }
  return ids.size;
});

const summary = computed(() => {
  if (relationFilter.value === 'all') {
    return `${nodes.value.length} nodes · ${edges.value.length} relations`;
  }
  return `${connectedNodeCount.value} connected nodes · ${matchingEdges.value.length} of ${edges.value.length} relations`;
});

function entityDestination(entity: GraphKey): RouteLocationRaw | null {
  switch (entity.kind) {
    case 'intent':
      return { name: 'intent-detail', params: { id: entity.id } };
    case 'scenario': {
      const parent = scenarioToIntent.value.get(entity.id);
      return parent
        ? { name: 'intent-detail', params: { id: parent }, hash: `#scenario-${entity.id}` }
        : null;
    }
    case 'notion':
      return { name: 'glossary', hash: `#notion-${entity.id}` };
    case 'constraint':
      return { name: 'coverage', hash: `#constraint-${entity.id}` };
    default:
      return null;
  }
}

const openDestination = computed<RouteLocationRaw | null>(() => {
  if (!selected.value) return null;
  if (selected.value.type === 'node') return entityDestination(selected.value.entity);

  const source = entityDestination(selected.value.source);
  const target = entityDestination(selected.value.target);
  if (source && !target) return source;
  if (target && !source) return target;
  return null;
});

function relationLabel(relation: GraphRelation): string {
  return relation.charAt(0).toUpperCase() + relation.slice(1);
}
</script>

<template>
  <section class="page graph-page">
    <div class="graph-page__heading">
      <div>
        <h1>Graph</h1>
        <p class="graph-page__summary">{{ summary }}</p>
      </div>

      <label v-if="edges.length" class="relation-filter">
        <span>Relation</span>
        <select v-model="relationFilter" aria-label="Filter graph by relation">
          <option value="all">All</option>
          <option v-for="relation in relations" :key="relation" :value="relation">
            {{ relationLabel(relation) }}
          </option>
        </select>
      </label>
    </div>

    <EmptyState
      v-if="nodes.length === 0"
      title="No graph nodes yet"
      text="Declare notions, intents, scenarios or bindings to build the project graph."
    />

    <template v-else>
      <p v-if="edges.length === 0" class="graph-page__notice">
        This graph has nodes but no relations yet.
      </p>

      <div class="graph-page__workspace">
        <CytoGraph
          :nodes="nodes"
          :edges="edges"
          :relation-filter="relationFilter"
          @select="selected = $event"
        />

        <aside class="selection-panel" aria-live="polite">
          <template v-if="selected?.type === 'node'">
            <p class="selection-panel__eyebrow">Selected node</p>
            <KindPill :kind="selected.entity.kind" />
            <h2>{{ selected.label }}</h2>
            <p class="selection-panel__id">{{ selected.entity.id }}</p>
            <RouterLink v-if="openDestination" :to="openDestination" class="selection-panel__open">
              Open
            </RouterLink>
          </template>

          <template v-else-if="selected?.type === 'edge'">
            <p class="selection-panel__eyebrow">Selected relation</p>
            <h2>{{ relationLabel(selected.relation) }}</h2>
            <dl class="selection-panel__edge">
              <div>
                <dt>Source</dt>
                <dd><EntityLink :entity="selected.source" /></dd>
              </div>
              <div>
                <dt>Target</dt>
                <dd><EntityLink :entity="selected.target" /></dd>
              </div>
            </dl>
            <RouterLink v-if="openDestination" :to="openDestination" class="selection-panel__open">
              Open
            </RouterLink>
          </template>

          <template v-else>
            <p class="selection-panel__eyebrow">Selection</p>
            <h2>Explore the graph</h2>
            <p class="selection-panel__hint">
              Select a node or relation to inspect it. Select the background to clear.
            </p>
          </template>
        </aside>
      </div>
    </template>
  </section>
</template>

<style scoped>
.graph-page__heading {
  display: flex;
  flex-wrap: wrap;
  align-items: end;
  justify-content: space-between;
  gap: 1rem;
  margin-bottom: 1rem;
}

.graph-page__heading h1 {
  margin-bottom: 0.125rem;
}

.graph-page__summary,
.graph-page__notice,
.selection-panel__hint {
  color: var(--color-text-muted);
}

.graph-page__summary {
  margin: 0;
  font-size: 0.875rem;
}

.graph-page__notice {
  margin-bottom: 0.75rem;
  border-left: 3px solid var(--color-status-warn);
  padding-left: 0.75rem;
}

.relation-filter {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  color: var(--color-text-muted);
  font-size: 0.875rem;
  font-weight: 600;
}

.relation-filter select {
  min-width: 9rem;
  border: 1px solid var(--color-border);
  border-radius: 0.375rem;
  background: var(--color-surface);
  padding: 0.375rem 2rem 0.375rem 0.625rem;
  color: var(--color-text);
}

.graph-page__workspace {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(15rem, 18rem);
  align-items: start;
  gap: 1rem;
}

.selection-panel {
  min-height: 12rem;
  border: 1px solid var(--color-border);
  border-radius: 0.75rem;
  background: var(--color-surface);
  padding: 1rem;
}

.selection-panel__eyebrow {
  margin-bottom: 0.5rem;
  color: var(--color-text-muted);
  font-size: 0.75rem;
  font-weight: 700;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.selection-panel h2 {
  margin: 0.5rem 0 0.375rem;
  font-size: 1rem;
}

.selection-panel__id {
  overflow-wrap: anywhere;
  color: var(--color-text-muted);
  font-family:
    ui-monospace,
    SFMono-Regular,
    Menlo,
    Consolas,
    'Liberation Mono',
    monospace;
  font-size: 0.8125rem;
}

.selection-panel__edge {
  margin: 0;
}

.selection-panel__edge div + div {
  margin-top: 0.75rem;
}

.selection-panel__edge dt {
  color: var(--color-text-muted);
  font-size: 0.75rem;
  font-weight: 600;
}

.selection-panel__edge dd {
  margin: 0.25rem 0 0;
  overflow-wrap: anywhere;
}

.selection-panel__open {
  display: inline-block;
  margin-top: 1rem;
  border-radius: 0.375rem;
  background: var(--color-primary);
  padding: 0.375rem 0.75rem;
  color: var(--color-text-on-primary);
  font-weight: 600;
}

.selection-panel__open:hover {
  background: var(--color-primary-strong);
  text-decoration: none;
}

@media (max-width: 52rem) {
  .graph-page__workspace {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
