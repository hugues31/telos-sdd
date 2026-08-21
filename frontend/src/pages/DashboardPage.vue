<script setup lang="ts">
import { computed } from 'vue';

import EmptyState from '../components/EmptyState.vue';
import MetricCard from '../components/MetricCard.vue';
import ProgressBar from '../components/ProgressBar.vue';
import StatusBadge from '../components/StatusBadge.vue';
import { snapshot } from '../data/snapshot';
import type { IntentStatus, ProjectState } from '../data/types';

const dashboard = computed(() => snapshot.value.snapshot.dashboard);
const coverage = computed(() => snapshot.value.snapshot.coverage);
const intents = computed(() => snapshot.value.snapshot.intents);

// Nothing declared at all yet — an empty (or brand new) project. The
// project-state section above still renders in this case (it's meaningful
// even with zero entities: a fresh project is "coherent" by definition),
// but the entity inventory becomes an EmptyState instead of a wall of zeros.
const isEmpty = computed(
  () =>
    coverage.value.notions === 0 &&
    coverage.value.intents_total === 0 &&
    coverage.value.constraints === 0,
);

const intentStatuses: IntentStatus[] = ['active', 'draft', 'deprecated'];

const intentStatusCounts = computed(() => {
  const counts: Record<IntentStatus, number> = { draft: 0, active: 0, deprecated: 0 };
  for (const intent of intents.value) {
    counts[intent.status] += 1;
  }
  return counts;
});

// Same ratio the Coverage page will detail per-row; here it's the single
// headline number for "how much of the model is proved by a test".
const scenarioCoveragePct = computed(() => {
  const total = coverage.value.scenarios_total;
  return total === 0 ? 0 : Math.round((coverage.value.scenarios_proved / total) * 100);
});

const stateInfo: Record<ProjectState, { label: string; description: string }> = {
  coherent: {
    label: 'Coherent',
    description: 'The model, the code and the tests all agree — nothing to reconcile.',
  },
  changing: {
    label: 'Changing',
    description: 'A change is in progress; some obligations are still open.',
  },
  drifted: {
    label: 'Drifted',
    description: 'Code or tests have drifted away from the model — see the drift below.',
  },
};

const currentState = computed(() => stateInfo[dashboard.value.state]);
</script>

<template>
  <section class="page dashboard">
    <h1>Dashboard</h1>

    <section class="dashboard__project-state" :class="`dashboard__project-state--${dashboard.state}`">
      <div class="dashboard__project-state-heading">
        <span class="dashboard__project-state-dot" aria-hidden="true"></span>
        <h2>{{ currentState.label }}</h2>
      </div>
      <p class="dashboard__project-state-description">{{ currentState.description }}</p>

      <ul v-if="dashboard.drift.length" class="dashboard__list">
        <li v-for="entry in dashboard.drift" :key="entry.path">
          <code>{{ entry.path }}</code> — {{ entry.kind }}
        </li>
      </ul>

      <ul v-if="dashboard.open_changes.length" class="dashboard__list">
        <li v-for="change in dashboard.open_changes" :key="change.id">
          <strong>{{ change.id }}</strong> ({{ change.status }})
          <ul v-if="change.obligations.length" class="dashboard__list">
            <li v-for="obligation in change.obligations" :key="obligation">{{ obligation }}</li>
          </ul>
        </li>
      </ul>
    </section>

    <EmptyState
      v-if="isEmpty"
      title="No project data yet"
      text="Declare notions, intents and constraints in your .tel files to see them here."
    />

    <template v-else>
      <div class="dashboard__metrics">
        <MetricCard label="Intents" :value="coverage.intents_total" to="/intents">
          <div class="dashboard__status-breakdown">
            <span v-for="status in intentStatuses" :key="status" class="dashboard__status-count">
              <StatusBadge :status="status" /> {{ intentStatusCounts[status] }}
            </span>
          </div>
        </MetricCard>

        <MetricCard
          label="Scenarios"
          :value="coverage.scenarios_total"
          :subtext="`${coverage.scenarios_proved} proved`"
        />

        <MetricCard label="Notions" :value="coverage.notions" to="/glossary" />

        <MetricCard label="Constraints" :value="coverage.constraints" to="/coverage" />
      </div>

      <MetricCard
        label="Scenario proof coverage"
        :value="`${scenarioCoveragePct}%`"
        :subtext="`${coverage.scenarios_proved} of ${coverage.scenarios_total} scenarios proved`"
        to="/coverage"
      >
        <ProgressBar
          :value="scenarioCoveragePct"
          color="--color-primary"
          label="Scenario proof coverage"
        />
      </MetricCard>
    </template>
  </section>
</template>

<style scoped>
.dashboard__project-state {
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  border-radius: 0.75rem;
  padding: 1.25rem 1.5rem;
  margin-bottom: 1.5rem;
}

.dashboard__project-state-heading {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.dashboard__project-state-heading h2 {
  margin: 0;
  font-size: 1.125rem;
}

.dashboard__project-state-dot {
  width: 0.625rem;
  height: 0.625rem;
  border-radius: 50%;
  flex-shrink: 0;
  background: var(--color-status-ok);
}

.dashboard__project-state--changing .dashboard__project-state-dot {
  background: var(--color-status-warn);
}

.dashboard__project-state--drifted .dashboard__project-state-dot {
  background: var(--color-status-error);
}

.dashboard__project-state-description {
  color: var(--color-text-muted);
  margin: 0.5rem 0 0;
}

.dashboard__list {
  margin: 0.75rem 0 0;
  padding-left: 1.25rem;
}

.dashboard__metrics {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.dashboard__status-breakdown {
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
}

.dashboard__status-count {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  font-size: 0.8125rem;
  color: var(--color-text-muted);
}
</style>
