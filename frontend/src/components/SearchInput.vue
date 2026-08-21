<script setup lang="ts">
// Controlled search field: filtering happens as a computed in the list page
// that owns the data, so this just forwards every keystroke immediately —
// no debounce.
withDefaults(
  defineProps<{
    modelValue: string;
    /** Required: the field has no visible label, only this. */
    ariaLabel: string;
    placeholder?: string;
  }>(),
  { placeholder: undefined },
);

const emit = defineEmits<{
  'update:modelValue': [value: string];
}>();

function onInput(event: Event): void {
  emit('update:modelValue', (event.target as HTMLInputElement).value);
}

function clear(): void {
  emit('update:modelValue', '');
}
</script>

<template>
  <div class="search-input">
    <svg class="search-input__icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <circle cx="10.5" cy="10.5" r="6.5" stroke="currentColor" stroke-width="1.5" fill="none" />
      <line x1="15.3" y1="15.3" x2="20.5" y2="20.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
    </svg>
    <input
      type="text"
      class="search-input__field"
      :value="modelValue"
      :placeholder="placeholder"
      :aria-label="ariaLabel"
      @input="onInput"
    />
    <button
      v-if="modelValue"
      type="button"
      class="search-input__clear"
      aria-label="Clear search"
      @click="clear"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
        <line x1="6" y1="6" x2="18" y2="18" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
        <line x1="18" y1="6" x2="6" y2="18" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.search-input {
  position: relative;
  display: flex;
  align-items: center;
}

.search-input__icon {
  position: absolute;
  left: 0.625rem;
  width: 1rem;
  height: 1rem;
  color: var(--color-text-muted);
  pointer-events: none;
}

.search-input__field {
  width: 100%;
  padding: 0.5rem 2rem;
  border: 1px solid var(--color-border);
  border-radius: 0.5rem;
  background: var(--color-surface);
  color: var(--color-text);
}

.search-input__clear {
  position: absolute;
  right: 0.375rem;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 1.5rem;
  height: 1.5rem;
  border: none;
  border-radius: 0.25rem;
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
}

.search-input__clear:hover {
  color: var(--color-text);
  background: var(--color-bg-subtle);
}

.search-input__clear svg {
  width: 0.875rem;
  height: 0.875rem;
}
</style>
