<script setup lang="ts">
import { useRoute } from 'vue-router';

const route = useRoute();
</script>

<template>
  <section class="page">
    <h1>Intent {{ route.params.id }}</h1>
    <div class="empty-state">The full intent detail will show up here in a later task.</div>
  </section>
</template>
