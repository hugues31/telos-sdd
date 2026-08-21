<script setup lang="ts">
// Route: /intent/:id. Layout order matches the brief: the `telos` line is
// the lede, up top before anything else; the canonical `.tel` source sits
// behind a closed <details> (its `<pre class="tel-source">` is the one
// substitution point a later task swaps for a TelCode syntax-highlighting
// component — nothing else on this page touches `.canonical`); then
// relations, notions, constraints, implementations and scenarios each get
// their own section.
import { computed } from 'vue';
import { useRoute } from 'vue-router';

import EmptyState from '../components/EmptyState.vue';
import EntityLink from '../components/EntityLink.vue';
import StatusBadge from '../components/StatusBadge.vue';
import { intentById, snapshot } from '../data/snapshot';
import type { GraphEdgeView, GraphKey, GraphRelation } from '../data/types';

const route = useRoute();

const intentId = computed(() => {
  const raw = route.params.id;
  return Array.isArray(raw) ? (raw[0] ?? '') : raw;
});

const intent = computed(() => intentById.value.get(intentId.value));

// The "Relations" section below shows edges touching this intent whose
// relation kind has no dedicated, more complete section further down:
// `uses` -> Notions, `constrains` -> Constraints (which must include
// *global* constraints too — those never get a `constrains` edge, see the
// snapshot builder comment in public/data.js — so it reads intent.constraints
// directly rather than edges), `verifies` -> Scenarios, `implements` ->
// Implementations. What's left standing (currently just `requires`) is the
// one relation IntentView exposes no typed field for at all.
const COVERED_RELATIONS = new Set<GraphRelation>(['uses', 'constrains', 'verifies', 'implements']);

interface RelationGroup {
  relation: GraphRelation;
  entities: GraphKey[];
}

function groupByRelation(
  edges: GraphEdgeView[],
  pick: (edge: GraphEdgeView) => GraphKey,
): RelationGroup[] {
  const map = new Map<GraphRelation, GraphKey[]>();
  for (const edge of edges) {
    const group = map.get(edge.relation);
    if (group) {
      group.push(pick(edge));
    } else {
      map.set(edge.relation, [pick(edge)]);
    }
  }
  return [...map.entries()].map(([relation, entities]) => ({ relation, entities }));
}

const outgoingRelations = computed<RelationGroup[]>(() => {
  const edges = snapshot.value.snapshot.edges.filter(
    (edge) =>
      edge.from.kind === 'intent' &&
      edge.from.id === intentId.value &&
      !COVERED_RELATIONS.has(edge.relation),
  );
  return groupByRelation(edges, (edge) => edge.to);
});

const incomingRelations = computed<RelationGroup[]>(() => {
  const edges = snapshot.value.snapshot.edges.filter(
    (edge) =>
      edge.to.kind === 'intent' &&
      edge.to.id === intentId.value &&
      !COVERED_RELATIONS.has(edge.relation),
  );
  return groupByRelation(edges, (edge) => edge.from);
});

function relationLabel(relation: GraphRelation): string {
  return relation.charAt(0).toUpperCase() + relation.slice(1);
}
</script>

<template>
  <section class="page intent-detail">
    <template v-if="intent">
      <div class="intent-detail__heading">
        <h1>{{ intent.id }} — {{ intent.title }}</h1>
        <StatusBadge :status="intent.status" />
      </div>
      <p class="intent-detail__telos">{{ intent.telos }}</p>

      <details class="intent-detail__source">
        <summary>Canonical .tel source</summary>
        <!-- Single substitution point: a later task replaces this <pre>
             with a TelCode syntax-highlighting component. -->
        <pre class="tel-source">{{ intent.canonical }}</pre>
      </details>

      <section class="intent-detail__section" aria-labelledby="relations-heading">
        <h2 id="relations-heading">Relations</h2>
        <template v-if="outgoingRelations.length || incomingRelations.length">
          <div v-if="outgoingRelations.length" class="relation-group">
            <h3>Outgoing</h3>
            <div
              v-for="group in outgoingRelations"
              :key="`out-${group.relation}`"
              class="relation-group__row"
            >
              <span class="relation-group__label">{{ relationLabel(group.relation) }}</span>
              <EntityLink
                v-for="entity in group.entities"
                :key="`${entity.kind}-${entity.id}`"
                :entity="entity"
              />
            </div>
          </div>
          <div v-if="incomingRelations.length" class="relation-group">
            <h3>Incoming</h3>
            <div
              v-for="group in incomingRelations"
              :key="`in-${group.relation}`"
              class="relation-group__row"
            >
              <span class="relation-group__label">{{ relationLabel(group.relation) }}</span>
              <EntityLink
                v-for="entity in group.entities"
                :key="`${entity.kind}-${entity.id}`"
                :entity="entity"
              />
            </div>
          </div>
        </template>
        <p v-else class="intent-detail__muted">No other intent relations.</p>
      </section>

      <section class="intent-detail__section" aria-labelledby="notions-heading">
        <h2 id="notions-heading">Notions</h2>
        <ul v-if="intent.notions.length" class="entity-chip-list">
          <li v-for="name in intent.notions" :key="name">
            <EntityLink :entity="{ kind: 'notion', id: name }" :show-kind="false" />
          </li>
        </ul>
        <p v-else class="intent-detail__muted">No notions used.</p>
      </section>

      <section class="intent-detail__section" aria-labelledby="constraints-heading">
        <h2 id="constraints-heading">Constraints</h2>
        <ul v-if="intent.constraints.length" class="entity-chip-list">
          <li v-for="constraint in intent.constraints" :key="constraint.id">
            <EntityLink :entity="{ kind: 'constraint', id: constraint.id }" :show-kind="false" />
            <span class="entity-chip-list__extra">{{ constraint.title }}</span>
          </li>
        </ul>
        <p v-else class="intent-detail__muted">No constraints attached.</p>
      </section>

      <section class="intent-detail__section" aria-labelledby="implementations-heading">
        <h2 id="implementations-heading">Implementations</h2>
        <ul v-if="intent.implements.length" class="entity-chip-list">
          <li v-for="path in intent.implements" :key="path">
            <EntityLink :entity="{ kind: 'code', id: path }" />
          </li>
        </ul>
        <p v-else class="intent-detail__muted">Not implemented yet.</p>
      </section>

      <section class="intent-detail__section" aria-labelledby="scenarios-heading">
        <h2 id="scenarios-heading">Scenarios</h2>
        <article
          v-for="scenario in intent.scenarios"
          :id="`scenario-${scenario.id}`"
          :key="scenario.id"
          class="scenario-card"
        >
          <h3 class="scenario-card__title">
            <span class="scenario-card__id">{{ scenario.id }}</span>
            {{ scenario.title }}
          </h3>
          <p v-if="scenario.proves.length" class="scenario-card__proof">
            Proved by
            <EntityLink
              v-for="test in scenario.proves"
              :key="test"
              :entity="{ kind: 'test', id: test }"
              :show-kind="false"
            />
          </p>
          <p v-else class="scenario-card__proof scenario-card__proof--unproved">
            Not proved by a test yet.
          </p>
        </article>
        <p v-if="!intent.scenarios.length" class="intent-detail__muted">No scenarios yet.</p>
      </section>
    </template>

    <template v-else>
      <h1>Intent not found</h1>
      <EmptyState
        title="Unknown intent id"
        :text="`'${intentId}' doesn't match any intent in this snapshot.`"
      >
        <RouterLink to="/intents">← Back to intents</RouterLink>
      </EmptyState>
    </template>
  </section>
</template>

<style scoped>
.intent-detail__heading {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.75rem;
}

.intent-detail__heading h1 {
  margin: 0;
}

.intent-detail__telos {
  font-size: 1.125rem;
  color: var(--color-text);
  border-left: 3px solid var(--color-primary);
  padding-left: 0.75rem;
  margin: 0.75rem 0 1.5rem;
}

.intent-detail__source {
  margin-bottom: 1.5rem;
}

.intent-detail__source summary {
  cursor: pointer;
  color: var(--color-link);
  font-weight: 600;
}

.tel-source {
  margin: 0.75rem 0 0;
  padding: 1rem;
  background: var(--color-bg-subtle);
  border: 1px solid var(--color-border);
  border-radius: 0.5rem;
  overflow-x: auto;
  font-size: 0.8125rem;
  line-height: 1.5;
}

.intent-detail__section {
  margin-bottom: 1.75rem;
}

.intent-detail__section h2 {
  font-size: 1rem;
  margin-bottom: 0.5rem;
}

.intent-detail__muted {
  color: var(--color-text-muted);
  margin: 0;
}

.relation-group {
  margin-bottom: 0.75rem;
}

.relation-group:last-child {
  margin-bottom: 0;
}

.relation-group h3 {
  font-size: 0.8125rem;
  font-weight: 600;
  color: var(--color-text-muted);
  text-transform: uppercase;
  letter-spacing: 0.03em;
  margin: 0 0 0.375rem;
}

.relation-group__row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem 0.75rem;
  margin-bottom: 0.375rem;
}

.relation-group__label {
  font-size: 0.8125rem;
  font-weight: 600;
  color: var(--color-text-muted);
  min-width: 5.5rem;
}

.entity-chip-list {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem 1rem;
  list-style: none;
  margin: 0;
  padding: 0;
}

.entity-chip-list li {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
}

.entity-chip-list__extra {
  color: var(--color-text-muted);
  font-size: 0.875rem;
}

.scenario-card {
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: 0.75rem;
  padding: 1rem 1.25rem;
  margin-bottom: 0.75rem;
  scroll-margin-top: var(--header-height);
}

.scenario-card__title {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0.5rem;
  font-size: 0.9375rem;
  margin: 0 0 0.5rem;
}

.scenario-card__id {
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

.scenario-card__proof {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem;
  margin: 0;
  font-size: 0.875rem;
  color: var(--color-text-muted);
}

.scenario-card__proof--unproved {
  color: var(--color-status-warn);
}
</style>
