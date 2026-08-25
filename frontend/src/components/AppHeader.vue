<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { useRoute } from 'vue-router';

import logoUrl from '../assets/logo.png';
import { snapshot } from '../data/snapshot';
import { shortcutLabel } from '../search/entities';
import GlobalSearch from './GlobalSearch.vue';
import ThemeToggle from './ThemeToggle.vue';

// Top-level, browsable destinations. `/intent/:id` (IntentDetailPage) is
// deliberately not listed here: it needs a concrete intent id, so it is only
// reachable by following a link from IntentsPage (or later, the graph), never
// from the nav itself.
const links = [
  { to: '/', label: 'Dashboard' },
  { to: '/contexts', label: 'Contexts' },
  { to: '/intents', label: 'Intents' },
  { to: '/graph', label: 'Graph' },
  { to: '/glossary', label: 'Glossary' },
  { to: '/coverage', label: 'Coverage' },
];

// Simple project-health heuristic: green when the snapshot's project state
// carries no error/inconsistency (`dashboard.state === 'coherent'`, i.e. no
// drift and no open change blocking it), orange for everything else
// ("changing" mid-edit, or "drifted" with unresolved drift). See
// `DashboardView.state` / `state_kind` in crates/telos/src/view/model.rs.
const projectState = computed(() => snapshot.value.snapshot.dashboard.state);
const isHealthy = computed(() => projectState.value === 'coherent');

const route = useRoute();
const header = ref<HTMLElement | null>(null);
const nav = ref<HTMLElement | null>(null);
const menuButton = ref<HTMLButtonElement | null>(null);
const globalSearch = ref<InstanceType<typeof GlobalSearch> | null>(null);
const menuOpen = ref(false);
const searchShortcut = shortcutLabel(typeof navigator === 'undefined' ? '' : navigator.platform);

async function toggleMenu(): Promise<void> {
  menuOpen.value = !menuOpen.value;
  if (menuOpen.value) {
    await nextTick();
    nav.value?.querySelector<HTMLAnchorElement>('a')?.focus();
  }
}

function closeMenu(returnFocus = false): void {
  if (!menuOpen.value) return;
  menuOpen.value = false;
  if (returnFocus) void nextTick(() => menuButton.value?.focus());
}

function openSearch(): void {
  closeMenu();
  void globalSearch.value?.open();
}

function onDocumentKeydown(event: KeyboardEvent): void {
  if (event.key !== 'Escape' || !menuOpen.value) return;
  event.preventDefault();
  closeMenu(true);
}

function onDocumentPointerdown(event: PointerEvent): void {
  if (!menuOpen.value || header.value?.contains(event.target as Node)) return;
  closeMenu();
}

watch(
  () => route.fullPath,
  () => closeMenu(),
);

onMounted(() => {
  document.addEventListener('keydown', onDocumentKeydown);
  document.addEventListener('pointerdown', onDocumentPointerdown);
});

onUnmounted(() => {
  document.removeEventListener('keydown', onDocumentKeydown);
  document.removeEventListener('pointerdown', onDocumentPointerdown);
});
</script>

<template>
  <header ref="header" class="app-header">
    <div class="app-header__inner">
      <RouterLink to="/" class="app-header__brand">
        <img
          class="app-header__logo"
          :src="logoUrl"
          width="128"
          height="37"
          alt="Telos"
        />
        <span
          class="app-header__status"
          :class="{ 'app-header__status--warn': !isHealthy }"
          :title="`Project state: ${projectState}`"
          :aria-label="`Project state: ${projectState}`"
          role="img"
        ></span>
      </RouterLink>

      <nav
        id="main-navigation"
        ref="nav"
        class="app-header__nav"
        :class="{ 'app-header__nav--open': menuOpen }"
        aria-label="Main"
      >
        <RouterLink v-for="link in links" :key="link.to" :to="link.to" class="app-header__link">
          {{ link.label }}
        </RouterLink>
      </nav>

      <div class="app-header__actions">
        <button
          type="button"
          class="app-header__search"
          aria-label="Search all Telos entities"
          title="Search all Telos entities"
          @click="openSearch"
        >
          <span class="app-header__search-icon" aria-hidden="true">⌕</span>
          <span class="app-header__search-label">Search</span>
          <kbd class="app-header__shortcut">{{ searchShortcut }}</kbd>
        </button>
        <ThemeToggle />
        <button
          ref="menuButton"
          type="button"
          class="app-header__menu-button"
          aria-controls="main-navigation"
          :aria-expanded="menuOpen"
          :aria-label="menuOpen ? 'Close main menu' : 'Open main menu'"
          @click="toggleMenu"
        >
          <span aria-hidden="true">{{ menuOpen ? '×' : '☰' }}</span>
        </button>
      </div>
    </div>
    <GlobalSearch ref="globalSearch" />
  </header>
</template>

<style scoped>
.app-header {
  position: sticky;
  top: 0;
  z-index: 10;
  height: var(--header-height);
  background: var(--color-surface);
  border-bottom: 1px solid var(--color-border);
}

.app-header__inner {
  position: relative;
  max-width: 72rem;
  height: 100%;
  margin: 0 auto;
  padding-inline: 1.5rem;
  display: flex;
  align-items: center;
  gap: 1.5rem;
}

.app-header__brand {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  color: var(--color-text);
}

.app-header__brand:hover {
  text-decoration: none;
}

.app-header__logo {
  display: block;
  width: 6rem;
  height: auto;
}

.app-header__status {
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  background: var(--color-status-ok);
}

.app-header__status--warn {
  background: var(--color-status-warn);
}

.app-header__nav {
  display: flex;
  gap: 1rem;
  flex: 1;
}

.app-header__actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.app-header__search,
.app-header__menu-button {
  display: inline-flex;
  min-width: 2.5rem;
  min-height: 2.5rem;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--color-border);
  border-radius: 0.5rem;
  background: var(--color-surface);
  cursor: pointer;
}

.app-header__search {
  gap: 0.5rem;
  padding-inline: 0.75rem;
  color: var(--color-text-muted);
}

.app-header__search:hover,
.app-header__menu-button:hover {
  color: var(--color-text);
  background: var(--color-bg-subtle);
}

.app-header__search-icon {
  color: var(--color-primary-strong);
  font-size: 1.2rem;
  line-height: 1;
}

.app-header__shortcut {
  padding: 0.125rem 0.375rem;
  color: var(--color-text-muted);
  background: var(--color-bg-subtle);
  border: 1px solid var(--color-border);
  border-radius: 0.25rem;
  font: inherit;
  font-size: 0.6875rem;
  white-space: nowrap;
}

.app-header__menu-button {
  display: none;
  padding: 0;
  font-size: 1.25rem;
  line-height: 1;
}

.app-header__link {
  color: var(--color-text-muted);
  padding-block: 0.25rem;
}

.app-header__link:hover {
  color: var(--color-text);
  text-decoration: none;
}

.app-header__link.router-link-active {
  color: var(--color-primary-strong);
  font-weight: 600;
}

@media (max-width: 48rem) {
  .app-header__inner {
    padding-inline: 0.75rem;
    gap: 0.75rem;
  }

  .app-header__brand {
    margin-right: auto;
  }

  .app-header__nav {
    position: absolute;
    top: calc(100% + 1px);
    right: 0.75rem;
    left: 0.75rem;
    display: none;
    padding: 0.5rem;
    background: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-top: 0;
    border-radius: 0 0 0.75rem 0.75rem;
    box-shadow: 0 0.75rem 2rem rgb(8 24 48 / 18%);
  }

  .app-header__nav--open {
    display: grid;
  }

  .app-header__link {
    min-height: 2.75rem;
    padding: 0.625rem 0.75rem;
    border-radius: 0.375rem;
  }

  .app-header__link:hover,
  .app-header__link.router-link-active {
    background: var(--color-primary-soft);
  }

  .app-header__search {
    padding: 0;
  }

  .app-header__search-label,
  .app-header__shortcut {
    display: none;
  }

  .app-header__menu-button {
    display: inline-flex;
  }
}

@media (max-width: 24rem) {
  .app-header__logo {
    width: 5rem;
  }
}
</style>
