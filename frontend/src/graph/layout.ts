import cytoscape, { type Core } from 'cytoscape';
import cytoscapeDagre from 'cytoscape-dagre';

cytoscape.use(cytoscapeDagre);

export const DAGRE_LAYOUT_OPTIONS = {
  name: 'dagre',
  rankDir: 'LR',
  animate: false,
  fit: true,
  padding: 32,
  nodeSep: 44,
  edgeSep: 16,
  rankSep: 84,
} as const;

export function runDagreLayout(cy: Core): void {
  cy.layout(DAGRE_LAYOUT_OPTIONS).run();
}
