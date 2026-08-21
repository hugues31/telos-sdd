<script setup lang="ts">
import { useRoute } from 'vue-router';

import EmptyState from '../components/EmptyState.vue';

const route = useRoute();
</script>

<template>
  <section class="page">
    <h1>Intent {{ route.params.id }}</h1>
    <EmptyState title="Coming soon" text="The full intent detail will show up here in a later task." />
  </section>
</template>
