import { createApp } from 'vue';

import App from './App.vue';
import { hasSnapshot } from './data/snapshot';
import { router } from './router';
import './styles/tokens.css';
import './styles/base.css';

const root = document.getElementById('app');

if (root && hasSnapshot) {
  createApp(App).use(router).mount(root);
} else if (root) {
  // window.__TELOS_DATA__ is missing — an abnormal case (a broken export, or
  // index.html opened without its data.js next to it). Render a minimal
  // page instead of mounting an app that has nothing to read, so this fails
  // loudly rather than as a blank screen.
  root.innerHTML = `
    <div class="page" role="alert">
      <h1>Telos</h1>
      <div class="empty-state">
        <p>No project data was found (<code>window.__TELOS_DATA__</code> is missing).</p>
        <p>Serve this page through <code>telos view</code>, a <code>telos view --export</code> output, or <code>npm run dev</code> for local development.</p>
      </div>
    </div>
  `;
}
