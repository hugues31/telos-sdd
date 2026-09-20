<script setup lang="ts">
import { computed } from 'vue';
import { snapshot } from '../data/snapshot';
import PlanCard from '../components/PlanCard.vue';
const plans = computed(() => [...snapshot.value.snapshot.plans].sort((a,b) => b.last_activity.localeCompare(a.last_activity)));
</script>
<template>
  <section class="page"><h1>Plans</h1><p>Approved scope, execution progress and retained history.</p>
    <p v-if="!plans.length">No plans yet.</p>
    <PlanCard v-for="plan in plans" :key="plan.id" :plan="plan" />
  </section>
</template>
