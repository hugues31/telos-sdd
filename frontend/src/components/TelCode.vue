<script setup lang="ts">
import { computed } from 'vue';

import { tokenize } from '../tel/tokenizer';

const props = defineProps<{
  source: string;
}>();

const tokens = computed(() => tokenize(props.source));
</script>

<template>
  <pre class="tel-code"><code><span v-for="(token, index) in tokens" :key="index" :class="`tel-${token.kind}`">{{ token.text }}</span></code></pre>
</template>

<style scoped>
.tel-code {
  margin: 0.75rem 0 0;
  padding: 1rem;
  background: var(--color-bg-subtle);
  border: 1px solid var(--color-border);
  border-radius: 0.5rem;
  overflow-x: auto;
  font-size: 0.8125rem;
  line-height: 1.5;
  white-space: pre;
}

.tel-keyword { color: var(--tel-keyword); }
.tel-ident { color: var(--tel-ident); }
.tel-idlit { color: var(--tel-idlit); }
.tel-string { color: var(--tel-string); }
.tel-number { color: var(--tel-number); }
.tel-date { color: var(--tel-date); }
.tel-punct { color: var(--tel-punct); }
.tel-plain { color: var(--tel-plain); }
</style>
