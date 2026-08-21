<script setup lang="ts">
import { computed } from 'vue';

import EmptyState from '../components/EmptyState.vue';
import EntityLink from '../components/EntityLink.vue';
import KindPill from '../components/KindPill.vue';
import MetricCard from '../components/MetricCard.vue';
import ProgressBar from '../components/ProgressBar.vue';
import TelCode from '../components/TelCode.vue';
import { snapshot } from '../data/snapshot';
import { percentage } from './page-data';

const model = computed(() => snapshot.value.snapshot);
const coverage = computed(() => model.value.coverage);
const activeIntentPct = computed(() => percentage(coverage.value.intents_active, coverage.value.intents_total));
const implementedIntentPct = computed(() => percentage(coverage.value.intents_implemented, coverage.value.intents_total));
const provedScenarioPct = computed(() => percentage(coverage.value.scenarios_proved, coverage.value.scenarios_total));
</script>

<template>
  <section class="page coverage">
    <h1>Coverage</h1>
    <section class="coverage__metrics" aria-label="Coverage metrics">
      <MetricCard label="Notions" :value="coverage.notions" />
      <MetricCard label="Constraints" :value="coverage.constraints" />
      <MetricCard label="Intents" :value="coverage.intents_total" />
      <MetricCard label="Scenarios" :value="coverage.scenarios_total" />
      <MetricCard label="Active intents" :value="coverage.intents_active" :subtext="`${coverage.intents_active} of ${coverage.intents_total}`"><ProgressBar :value="activeIntentPct" label="Active intent coverage" /></MetricCard>
      <MetricCard label="Implemented intents" :value="coverage.intents_implemented" :subtext="`${coverage.intents_implemented} of ${coverage.intents_total}`"><ProgressBar :value="implementedIntentPct" label="Implemented intent coverage" /></MetricCard>
      <MetricCard label="Proved scenarios" :value="coverage.scenarios_proved" :subtext="`${coverage.scenarios_proved} of ${coverage.scenarios_total}`"><ProgressBar :value="provedScenarioPct" label="Scenario proof coverage" /></MetricCard>
    </section>

    <section class="coverage__section" aria-labelledby="coverage-matrix-heading">
      <h2 id="coverage-matrix-heading">Coverage matrix</h2>
      <EmptyState v-if="!coverage.rows.length" title="No coverage rows" text="Add scenarios to populate the matrix." />
      <div v-else class="coverage__table-wrap" tabindex="0" aria-label="Scrollable coverage matrix">
        <table><caption class="sr-only">Intent scenarios and their proof tests</caption><thead><tr><th scope="col">Intent</th><th scope="col">Scenario</th><th scope="col">Proof / test</th></tr></thead><tbody>
          <tr v-for="row in coverage.rows" :key="`${row.intent}-${row.scenario}-${row.test ?? 'none'}`" :class="{ 'coverage__row--unproved': row.test === null }"><td><EntityLink :entity="{ kind: 'intent', id: row.intent }" :show-kind="false" /></td><td><EntityLink :entity="{ kind: 'scenario', id: row.scenario }" :show-kind="false" /></td><td v-if="row.test"><code>{{ row.test }}</code></td><td v-else><span class="coverage__no-proof">No proof</span></td></tr>
        </tbody></table>
      </div>
    </section>

    <section class="coverage__section" aria-labelledby="implementations-heading"><h2 id="implementations-heading">Implementation bindings</h2><EmptyState v-if="!model.implementations.length" title="No implementation bindings" text="Link an intent to source code to show it here." /><ul v-else class="coverage__bindings"><li v-for="binding in model.implementations" :key="`${binding.intent}-${binding.path}`"><EntityLink :entity="{ kind: 'code', id: binding.path }" :show-kind="false" /><span aria-hidden="true">→</span><EntityLink :entity="{ kind: 'intent', id: binding.intent }" :show-kind="false" /></li></ul></section>
    <section class="coverage__section" aria-labelledby="proofs-heading"><h2 id="proofs-heading">Proof bindings</h2><EmptyState v-if="!model.proofs.length" title="No proof bindings" text="Attach a test to a scenario to show it here." /><ul v-else class="coverage__bindings"><li v-for="binding in model.proofs" :key="`${binding.scenario}-${binding.test}`"><EntityLink :entity="{ kind: 'test', id: binding.test }" :show-kind="false" /><span aria-hidden="true">→</span><EntityLink :entity="{ kind: 'scenario', id: binding.scenario }" :show-kind="false" /></li></ul></section>

    <section class="coverage__section" aria-labelledby="constraints-heading">
      <h2 id="constraints-heading">Constraints</h2>
      <EmptyState v-if="!model.constraints.length" title="No constraints" text="Declare constraints in .tel files to show them here." />
      <article v-for="constraint in model.constraints" :id="`constraint-${constraint.id}`" :key="constraint.id" class="constraint-card">
        <header class="constraint-card__header"><div><h3>{{ constraint.title }}</h3><p class="constraint-card__id">{{ constraint.id }}</p></div><KindPill :kind="constraint.kind" /></header>
        <p><strong>Scope:</strong> {{ constraint.scope }}</p>
        <details><summary>Canonical .tel source</summary><TelCode :source="constraint.canonical" /></details>
      </article>
    </section>
  </section>
</template>

<style scoped>
.coverage__metrics { display: grid; grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr)); gap: 1rem; }
.coverage__section { margin-top: 2rem; }
.coverage__table-wrap { overflow-x: auto; border: 1px solid var(--color-border); border-radius: 0.75rem; }
.coverage__table-wrap:focus-visible { outline-offset: 0.25rem; }
.coverage__row--unproved { background: var(--color-status-warn-bg); }
.coverage__no-proof { color: var(--color-status-warn); font-weight: 600; }
.coverage__bindings { display: grid; gap: 0.5rem; margin: 0; padding: 0; list-style: none; }
.coverage__bindings li { display: flex; flex-wrap: wrap; align-items: center; gap: 0.5rem; padding: 0.75rem 1rem; background: var(--color-surface); border: 1px solid var(--color-border); border-radius: 0.5rem; }
.constraint-card { scroll-margin-top: calc(var(--header-height) + 1rem); padding: 1.25rem; background: var(--color-surface); border: 1px solid var(--color-border); border-radius: 0.75rem; }
.constraint-card + .constraint-card { margin-top: 1rem; }
.constraint-card__header { display: flex; flex-wrap: wrap; align-items: start; justify-content: space-between; gap: 0.75rem; }
.constraint-card__header h3, .constraint-card__id { margin: 0; }
.constraint-card__id { color: var(--color-text-muted); font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace; font-size: 0.875rem; }
.constraint-card summary { cursor: pointer; color: var(--color-link); font-weight: 600; }
</style>
<style>.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }</style>
