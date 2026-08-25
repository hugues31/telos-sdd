<script setup lang="ts">
import { computed } from 'vue';

import EmptyState from '../components/EmptyState.vue';
import ProgressBar from '../components/ProgressBar.vue';
import { snapshot } from '../data/snapshot';

const contexts = computed(() => snapshot.value.snapshot.contexts);

function coverage(proved: number, scenarios: number): number {
  return scenarios === 0 ? 100 : Math.round((proved / scenarios) * 100);
}
</script>

<template>
  <section class="page contexts-page">
    <header class="contexts-page__heading">
      <div>
        <h1>Bounded contexts</h1>
        <p>Explicit ownership, capabilities, dependencies and translation boundaries.</p>
      </div>
      <RouterLink to="/graph">Open relation graph</RouterLink>
    </header>

    <EmptyState
      v-if="contexts.length === 0"
      title="No bounded contexts"
      text="Declare a context and its capabilities to make domain ownership executable."
    />

    <div v-else class="context-grid">
      <article
        v-for="context in contexts"
        :id="`context-${context.id}`"
        :key="context.id"
        class="context-card"
      >
        <header class="context-card__header">
          <div>
            <span class="context-card__kind">{{ context.kind }}</span>
            <h2>{{ context.title }}</h2>
            <code>CTX:{{ context.id }}</code>
          </div>
          <dl class="context-card__health">
            <div><dt>Intents</dt><dd>{{ context.health.active_intents }}/{{ context.health.intents }} active</dd></div>
            <div><dt>Proofs</dt><dd>{{ context.health.proved_scenarios }}/{{ context.health.scenarios }}</dd></div>
          </dl>
        </header>
        <p>{{ context.definition }}</p>
        <ProgressBar
          :value="coverage(context.health.proved_scenarios, context.health.scenarios)"
          :label="`${context.title} scenario proof coverage`"
        />

        <section class="context-card__section">
          <h3>Capabilities</h3>
          <ul v-if="context.capabilities.length" class="capability-list">
            <li
              v-for="capability in context.capabilities"
              :id="`capability-${capability.id.replace('/', '-')}`"
              :key="capability.id"
            >
              <div><strong>{{ capability.title }}</strong> <code>CAP:{{ capability.id }}</code></div>
              <p>{{ capability.definition }}</p>
            </li>
          </ul>
          <p v-else class="muted">No capability declared.</p>
        </section>

        <section class="context-card__section">
          <h3>Dependencies and mappings</h3>
          <ul v-if="context.dependencies.length" class="dependency-list">
            <li v-for="dependency in context.dependencies" :key="dependency.supplier">
              <strong>depends on <code>CTX:{{ dependency.supplier }}</code></strong>
              <ul v-if="dependency.mappings.length">
                <li v-for="mapping in dependency.mappings" :key="`${mapping.from}-${mapping.to}`">
                  <code>{{ mapping.from }}</code> → <code>{{ mapping.to }}</code>
                </li>
              </ul>
            </li>
          </ul>
          <p v-else class="muted">No supplier dependency.</p>
        </section>
      </article>
    </div>
  </section>
</template>

<style scoped>
.contexts-page__heading, .context-card__header { display: flex; justify-content: space-between; align-items: flex-start; gap: 1rem; }
.contexts-page__heading { margin-bottom: 1.5rem; }
.contexts-page__heading h1, .context-card__header h2 { margin-bottom: 0.25rem; }
.contexts-page__heading p, .context-card p, .muted { color: var(--color-text-muted); }
.context-grid { display: grid; gap: 1.25rem; }
.context-card { scroll-margin-top: calc(var(--header-height) + 1rem); padding: 1.5rem; border: 1px solid var(--color-border); border-radius: 0.75rem; background: var(--color-surface); }
.context-card__kind { color: var(--color-text-muted); font-size: 0.75rem; font-weight: 700; letter-spacing: 0.08em; text-transform: uppercase; }
.context-card__health { display: flex; gap: 1.5rem; margin: 0; }
.context-card__health div { text-align: right; }
.context-card__health dt { color: var(--color-text-muted); font-size: 0.75rem; text-transform: uppercase; }
.context-card__health dd { margin: 0.2rem 0 0; font-weight: 700; }
.context-card__section { margin-top: 1.5rem; }
.context-card__section h3 { margin-bottom: 0.75rem; font-size: 1rem; }
.capability-list, .dependency-list { display: grid; gap: 0.75rem; margin: 0; padding: 0; list-style: none; }
.capability-list > li, .dependency-list > li { scroll-margin-top: calc(var(--header-height) + 1rem); padding: 0.875rem; border: 1px solid var(--color-border); border-radius: 0.5rem; background: var(--color-bg-subtle); }
.capability-list p { margin: 0.4rem 0 0; }
.dependency-list ul { margin-top: 0.5rem; }
code { font-size: 0.82em; }
@media (max-width: 720px) { .contexts-page__heading, .context-card__header { flex-direction: column; } .context-card__health div { text-align: left; } }
</style>
