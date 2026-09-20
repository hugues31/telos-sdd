<script setup lang="ts">
import type { PlanView } from '../data/types';
import ProgressBar from './ProgressBar.vue';
defineProps<{ plan: PlanView }>();
</script>

<template>
  <article class="plan-card">
    <div class="plan-card__heading"><h3>{{ plan.title }}</h3><span>{{ plan.state }}</span></div>
    <p>{{ plan.goal }}</p>
    <ProgressBar v-if="plan.progress.percent !== null" :value="plan.progress.percent" :label="`${plan.title} task completion`" color="--color-primary" />
    <p>{{ plan.progress.percent === null ? 'To be planned' : `${plan.progress.percent}% · ${plan.progress.done} of ${plan.progress.total} tasks done` }}</p>
    <p v-if="plan.progress.percent === 100 && plan.state !== 'completed'">Tasks complete · final validation pending</p>
    <p v-if="plan.current_task">Current task: <code>{{ plan.current_task }}</code></p>
    <RouterLink :to="`/plan/${plan.id}`">View plan →</RouterLink>
  </article>
</template>

<style scoped>
.plan-card { border: 1px solid var(--color-border); border-radius: .75rem; background: var(--color-surface); padding: 1.25rem; margin: 1rem 0; }
.plan-card__heading { display: flex; gap: 1rem; justify-content: space-between; align-items: baseline; }
h3 { margin: 0; }
p { color: var(--color-text-muted); }
</style>
