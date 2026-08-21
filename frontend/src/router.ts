// Hash history: the SPA must stay usable served from GitHub Pages *and*
// opened straight from disk as file://, and only hash routing works in both
// (see src/data/snapshot.ts / index.html for the same file:// constraint on
// the data layer).

import {
  createRouter,
  createWebHashHistory,
  type RouteRecordRaw,
  type RouterScrollBehavior,
} from 'vue-router';

// Pixels the sticky header covers, so an anchor scrolled to isn't hidden
// under it. Keep in sync with `--header-height` in src/styles/tokens.css
// (3.5rem @ the 16px root font-size set in base.css).
const HEADER_OFFSET = 56;

const routes: RouteRecordRaw[] = [
  { path: '/', name: 'dashboard', component: () => import('./pages/DashboardPage.vue') },
  { path: '/intents', name: 'intents', component: () => import('./pages/IntentsPage.vue') },
  {
    path: '/intent/:id',
    name: 'intent-detail',
    component: () => import('./pages/IntentDetailPage.vue'),
  },
  { path: '/graph', name: 'graph', component: () => import('./pages/GraphPage.vue') },
  { path: '/glossary', name: 'glossary', component: () => import('./pages/GlossaryPage.vue') },
  { path: '/coverage', name: 'coverage', component: () => import('./pages/CoveragePage.vue') },
];

const scrollBehavior: RouterScrollBehavior = (to, _from, savedPosition) => {
  if (savedPosition) {
    return savedPosition;
  }
  if (to.hash) {
    return { el: to.hash, top: HEADER_OFFSET };
  }
  return { top: 0 };
};

export const router = createRouter({
  history: createWebHashHistory(),
  routes,
  scrollBehavior,
});
