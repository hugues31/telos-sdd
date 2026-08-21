import vue from '@vitejs/plugin-vue';
import { defineConfig, type Plugin } from 'vitest/config';

// file:// treats module scripts as cross-origin, so the built page must load a
// classic deferred script instead of <script type="module">. The `crossorigin`
// attribute Vite also adds to its auto-injected stylesheet <link> hits the
// same problem under file:// (a `crossorigin`-mode fetch of a local file is
// blocked), so it gets stripped the same way.
function classicScript(): Plugin {
  return {
    name: 'telos:classic-script',
    apply: 'build',
    enforce: 'post',
    transformIndexHtml(html) {
      const transformed = html
        .replace(
          /<script type="module"(?: crossorigin)? src="([^"]+)"><\/script>/g,
          '<script defer src="$1"></script>',
        )
        .replace(
          /<link rel="stylesheet"(?: crossorigin)? href="([^"]+)">/g,
          '<link rel="stylesheet" href="$1">',
        );

      // Vite injects the entry into <head>. Keep the exported execution
      // contract explicit in source order as well: data.js first, then the
      // deferred classic IIFE.
      const entry = transformed.match(/\s*<script defer src="(\.\/assets\/app\.js)"><\/script>/);
      if (!entry) return transformed;

      return transformed
        .replace(entry[0], '')
        .replace(
          '<script src="./data.js"></script>',
          `<script src="./data.js"></script>\n    <script defer src="${entry[1]}"></script>`,
        );
    },
  };
}

export default defineConfig({
  base: './',
  plugins: [vue(), classicScript()],
  build: {
    sourcemap: false,
    // Required for the `assets/app.css` asset assetFileNames names below:
    // Rollup only extracts CSS into a real asset file for the `es`/`cjs`
    // output formats when cssCodeSplit is left at its default (true) — for
    // `iife` it silently falls back to injecting CSS via JS at runtime
    // instead, and drops it entirely from CSS-only chunks. cssCodeSplit:
    // false switches to Vite's other CSS path, which bundles everything
    // into one asset file unconditionally, independent of output.format.
    cssCodeSplit: false,
    // public/ only holds the dev fixture data.js; the exported data.js is
    // written by the telos binary next to index.html.
    copyPublicDir: false,
    modulePreload: { polyfill: false },
    rollupOptions: {
      output: {
        format: 'iife',
        inlineDynamicImports: true,
        entryFileNames: 'assets/app.js',
        assetFileNames: (info) =>
          (info.names ?? []).some((name) => name.endsWith('.css'))
            ? 'assets/app.css'
            : 'assets/[name][extname]',
      },
    },
  },
  test: {
    environment: 'node',
  },
});
