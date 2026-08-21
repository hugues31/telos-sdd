// Singleton theme state. The module-level `theme` ref is shared by every
// caller of `useTheme()`, so any component toggling the theme is
// immediately reflected everywhere else that reads it.
//
// The rule (mirrored by the anti-FOUC script in index.html, which must
// stay in sync with it): an explicit choice in localStorage always wins;
// only fall back to `prefers-color-scheme`, and only keep following it
// live while no explicit choice has been stored.

import { ref, type Ref } from 'vue';

export type Theme = 'light' | 'dark';

const STORAGE_KEY = 'telos-theme';

function readStoredTheme(): Theme | null {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return stored === 'light' || stored === 'dark' ? stored : null;
  } catch {
    return null;
  }
}

function applyTheme(next: Theme): void {
  document.documentElement.setAttribute('data-theme', next);
}

// The anti-FOUC script already set data-theme before this module ever
// loads; read it back rather than recomputing it, so the two can never
// disagree.
function currentDomTheme(): Theme {
  return document.documentElement.getAttribute('data-theme') === 'dark' ? 'dark' : 'light';
}

const theme = ref<Theme>(currentDomTheme());

function setTheme(next: Theme): void {
  theme.value = next;
  applyTheme(next);
  try {
    window.localStorage.setItem(STORAGE_KEY, next);
  } catch {
    // Storage unavailable (private mode, disabled) — the theme still
    // applies for this page load, it just won't persist.
  }
}

function toggleTheme(): void {
  setTheme(theme.value === 'dark' ? 'light' : 'dark');
}

let listening = false;

function ensureListeningToSystemChanges(): void {
  if (listening) return;
  listening = true;
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (event) => {
    if (readStoredTheme() != null) return; // an explicit choice overrides the system
    theme.value = event.matches ? 'dark' : 'light';
    applyTheme(theme.value);
  });
}

export interface UseTheme {
  theme: Ref<Theme>;
  setTheme: (next: Theme) => void;
  toggleTheme: () => void;
}

export function useTheme(): UseTheme {
  ensureListeningToSystemChanges();
  return { theme, setTheme, toggleTheme };
}
