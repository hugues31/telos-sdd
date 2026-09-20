<script setup lang="ts">
import { computed } from 'vue';
import { snapshot } from '../data/snapshot';
const props = defineProps<{ target: string }>();
const entries = computed(() => {
  const history = snapshot.value.snapshot.history;
  const latest = [...history].reverse().flatMap(r => [...r.entities].reverse()).find(e => e.selector === props.target || e.previous_selector === props.target || e.uid === props.target);
  const uids = new Set(latest ? [latest.uid] : []);
  return history.filter(r => r.plan === props.target || r.id === props.target || [...r.files,...r.observed_files].some(f => f.path === props.target) || r.entities.some(e => uids.has(e.uid))).slice().reverse();
});
</script>
<template>
  <section class="history"><h2>Change history</h2>
    <p v-if="!entries.length">No attributed changes recorded. Initial observations do not establish an implementation date.</p>
    <article v-for="entry in entries" :key="entry.id">
      <time :datetime="entry.at">{{ entry.at }}</time> · <RouterLink :to="`/plan/${entry.plan}`">View plan</RouterLink> · Revision {{ entry.revision }} · {{ entry.task }}
      <ul><li v-for="entity in entry.entities.filter(e => e.selector === target || e.previous_selector === target || entry.plan === target)" :key="`${entity.uid}:${entity.kind}:${entity.path}`"><strong>{{ entity.kind.replaceAll('_', ' ') }}</strong>: {{ entity.selector }} <code>{{ entity.path }}</code></li></ul>
      <details v-if="entry.observed_files.length"><summary>Recovery · {{ entry.observed_files.length }} observed changes</summary><ul><li v-for="file in entry.observed_files" :key="file.path"><code>{{ file.path }}</code></li></ul></details>
      <details><summary>{{ entry.files.length }} changed {{ entry.files.length === 1 ? 'file' : 'files' }} · {{ entry.id }}</summary><ul><li v-for="file in entry.files" :key="file.path"><code>{{ file.path }}</code> — {{ !file.before ? 'added' : !file.after ? 'removed' : 'modified' }}</li></ul></details>
    </article>
  </section>
</template>
<style scoped>
.history { margin-top: 1.5rem; }
article { padding: 1rem 0; border-bottom: 1px solid var(--color-border); }
</style>
