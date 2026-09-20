<script setup lang="ts">
import { computed } from 'vue';
import { useRoute } from 'vue-router';
import { snapshot } from '../data/snapshot';
import PlanCard from '../components/PlanCard.vue';
import HistoryPanel from '../components/HistoryPanel.vue';
const route = useRoute();
const plan = computed(() => snapshot.value.snapshot.plans.find(p => p.id === route.params.id));
const events = computed(() => [...(plan.value?.events ?? [])].reverse());
const checkpoint = computed(() => events.value.find(e => e.kind === 'checkpoint'));
</script>
<template>
  <section class="page plan-page">
    <RouterLink to="/plans">← All plans</RouterLink>
    <template v-if="plan">
      <h1>{{ plan.title }}</h1><PlanCard :plan="plan" />
      <p>Revision {{ plan.revision }} · {{ plan.approved ? 'Approved' : 'Awaiting approval' }} · Last activity <time :datetime="plan.last_activity">{{ plan.last_activity }}</time></p>
      <h2>Brief</h2><p>{{ plan.brief.summary }}</p>
      <ul><li v-for="criterion in plan.success_criteria" :key="criterion">{{ criterion }}</li></ul>
      <h3>Scope</h3><ul><li v-for="path in plan.scope" :key="path"><code>{{ path }}</code></li></ul>
      <h3 v-if="plan.brief.decisions.length">Decisions</h3>
      <ul><li v-for="decision in plan.brief.decisions" :key="decision.id">{{ decision.text }} — {{ decision.state }} ({{ decision.source }})</li></ul>
      <h3 v-if="plan.brief.questions.length">Questions</h3>
      <ul><li v-for="question in plan.brief.questions" :key="question.id">{{ question.text }} — {{ question.answer ?? (question.blocking ? 'Blocking · unanswered' : 'Open') }}</li></ul>
      <section v-if="checkpoint"><h2>Resume</h2><p>{{ checkpoint.data.summary }}</p><p>Next: {{ checkpoint.data.next_action }}</p></section>
      <p>Resume command: <code>telos plan resume {{ plan.id }} --json</code></p>
      <h2>Tasks</h2>
      <article v-for="task in plan.tasks" :key="task.id" class="task">
        <h3><code>{{ task.id }}</code> · {{ task.title }}</h3><p>{{ task.kind }} · {{ task.state.replaceAll('_', ' ') }}</p>
        <p v-if="task.blocker" role="status">Blocked: {{ task.blocker }}</p>
        <p v-if="task.depends_on.length">Depends on: {{ task.depends_on.join(', ') }}</p>
        <p v-if="task.change">Change: <code>{{ task.change }}</code></p>
        <p v-if="task.next_action">Next action: {{ task.next_action }}</p>
        <ul><li v-for="criterion in task.acceptance" :key="criterion">{{ criterion }}</li></ul>
        <details v-if="task.spec_delta"><summary>Approved specification delta</summary><pre>{{ task.spec_delta }}</pre></details>
        <details><summary>Allowed paths</summary><ul><li v-for="path in task.allowed_paths" :key="path"><code>{{ path }}</code></li></ul></details>
      </article>
      <HistoryPanel :target="plan.id" />
      <h2>Activity</h2><ol class="activity"><li v-for="event in events" :key="event.id">
        <time :datetime="event.at">{{ event.at }}</time> · {{ event.kind.replaceAll('_', ' ') }} <span v-if="event.task">· {{ event.task }}</span>
        <details><summary>Details</summary><pre>{{ JSON.stringify(event.data, null, 2) }}</pre></details>
      </li></ol>
    </template>
    <p v-else>Plan unavailable in this snapshot.</p>
  </section>
</template>
<style scoped>
.task { padding: 1rem; border: 1px solid var(--color-border); border-radius: .5rem; margin: 1rem 0; }
pre { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 30rem; overflow: auto; }
.activity li { margin: 1rem 0; }
</style>
