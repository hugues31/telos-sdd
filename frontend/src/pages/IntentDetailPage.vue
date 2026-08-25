<script setup lang="ts">
// Route: /intent/:id. Rust projects display-ready canonical fragments for
// the statement and scenarios, so this page can show complete behavior
// without parsing or reconstructing the .tel grammar in the frontend.
import { computed } from 'vue';
import { useRoute } from 'vue-router';

import EmptyState from '../components/EmptyState.vue';
import EntityLink from '../components/EntityLink.vue';
import StatusBadge from '../components/StatusBadge.vue';
import TelCode from '../components/TelCode.vue';
import { intentById, snapshot } from '../data/snapshot';
import type { GraphEdgeView, GraphKey, GraphRelation } from '../data/types';

const route = useRoute();

const intentId = computed(() => {
  const raw = route.params.id;
  return Array.isArray(raw) ? (raw[0] ?? '') : raw;
});

const intent = computed(() => intentById.value.get(intentId.value));
const ownerEntity = computed<GraphKey | null>(() => {
  if (!intent.value) return null;
  return {
    kind: intent.value.owner.includes('/') ? 'capability' : 'context',
    id: intent.value.owner,
  };
});

const RELATION_LABELS = {
  refines: { outgoing: 'Refines', incoming: 'Refined by' },
  requires: { outgoing: 'Requires', incoming: 'Required by' },
  excludes: { outgoing: 'Excludes', incoming: 'Excluded by' },
} as const;

type IntentRelation = keyof typeof RELATION_LABELS;
type RelationDirection = 'outgoing' | 'incoming';

function isIntentRelation(relation: GraphRelation): relation is IntentRelation {
  return relation in RELATION_LABELS;
}

interface RelationGroup {
  relation: IntentRelation;
  entities: GraphKey[];
}

function groupByRelation(
  edges: GraphEdgeView[],
  pick: (edge: GraphEdgeView) => GraphKey,
): RelationGroup[] {
  const map = new Map<IntentRelation, GraphKey[]>();
  for (const edge of edges) {
    if (!isIntentRelation(edge.relation)) continue;
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
      isIntentRelation(edge.relation),
  );
  return groupByRelation(edges, (edge) => edge.to);
});

const incomingRelations = computed<RelationGroup[]>(() => {
  const edges = snapshot.value.snapshot.edges.filter(
    (edge) =>
      edge.to.kind === 'intent' &&
      edge.to.id === intentId.value &&
      isIntentRelation(edge.relation),
  );
  return groupByRelation(edges, (edge) => edge.from);
});

function relationLabel(relation: IntentRelation, direction: RelationDirection): string {
  return RELATION_LABELS[relation][direction];
}
</script>

<template>
  <section class="page intent-detail">
    <template v-if="intent">
      <RouterLink to="/intents" class="intent-detail__back">← All intents</RouterLink>
      <div class="intent-detail__heading">
        <div>
          <h1>{{ intent.id }} — {{ intent.title }}</h1>
          <p v-if="ownerEntity" class="intent-detail__owner">
            Owner <EntityLink :entity="ownerEntity" :show-kind="false" />
          </p>
        </div>
        <StatusBadge :status="intent.status" />
      </div>
      <p class="intent-detail__telos">{{ intent.telos }}</p>

      <section class="statement-card" aria-labelledby="statement-heading">
        <header>
          <h2 id="statement-heading">Statement</h2>
          <span class="statement-card__template">{{ intent.statement.template }}</span>
        </header>
        <TelCode :source="intent.statement.canonical" />
      </section>

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
              <span class="relation-group__label">
                {{ relationLabel(group.relation, 'outgoing') }}
              </span>
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
              <span class="relation-group__label">
                {{ relationLabel(group.relation, 'incoming') }}
              </span>
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
          <TelCode :source="scenario.canonical" />
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

      <details class="intent-detail__source">
        <summary>Full canonical .tel source</summary>
        <TelCode :source="intent.canonical" />
      </details>
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
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.75rem;
}

.intent-detail__heading h1 {
  margin: 0;
}

.intent-detail__back {
  display: inline-block;
  margin-bottom: 0.75rem;
  font-size: 0.875rem;
}

.intent-detail__owner {
  display: flex;
  flex-wrap: wrap;
  gap: 0.375rem;
  margin: 0.25rem 0 0;
  color: var(--color-text-muted);
  font-size: 0.875rem;
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

.statement-card {
  margin-bottom: 1.75rem;
  padding: 1rem 1.25rem 1.25rem;
  background: var(--color-primary-soft);
  border: 1px solid color-mix(in srgb, var(--color-primary) 28%, var(--color-border));
  border-radius: 0.75rem;
}

.statement-card header {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
}

.statement-card h2 {
  margin: 0;
  font-size: 1rem;
}

.statement-card__template {
  padding: 0.1875rem 0.5rem;
  color: var(--color-primary-strong);
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: 999px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 0.75rem;
  font-weight: 700;
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
  margin: 0.875rem 0 0;
  font-size: 0.875rem;
  color: var(--color-text-muted);
}

.scenario-card__proof--unproved {
  color: var(--color-status-warn);
}

@media (max-width: 36rem) {
  .intent-detail__heading {
    display: grid;
  }

  .statement-card,
  .scenario-card {
    padding-inline: 0.875rem;
  }
}
</style>
