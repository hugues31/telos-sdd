import type cytoscape from 'cytoscape';

import type { GraphKeyKind } from '../data/types';

const KIND_SHAPES: Record<GraphKeyKind, cytoscape.Css.NodeShape> = {
  notion: 'ellipse',
  intent: 'round-rectangle',
  scenario: 'diamond',
  constraint: 'hexagon',
  code: 'rectangle',
  test: 'tag',
};

function cssToken(style: CSSStyleDeclaration, name: string): string {
  return style.getPropertyValue(name).trim();
}

export function buildGraphStylesheet(
  root: Element = document.documentElement,
): cytoscape.StylesheetJson {
  const tokens = getComputedStyle(root);
  const text = cssToken(tokens, '--color-text');
  const muted = cssToken(tokens, '--color-text-muted');
  const surface = cssToken(tokens, '--color-surface');
  const primary = cssToken(tokens, '--color-primary');

  const kindStyles: cytoscape.StylesheetJson = Object.entries(KIND_SHAPES).map(
    ([kind, shape]) => ({
      selector: `node[kind = "${kind}"]`,
      style: {
        shape,
        'background-color': cssToken(tokens, `--k-${kind}`),
      },
    }),
  );

  return [
    {
      selector: 'node',
      style: {
        label: 'data(label)',
        width: 44,
        height: 44,
        color: text,
        'font-size': 10,
        'font-weight': 600,
        'text-wrap': 'wrap',
        'text-max-width': '150px',
        'text-valign': 'bottom',
        'text-margin-y': 8,
        'text-background-color': surface,
        'text-background-opacity': 0.9,
        'text-background-padding': '2px',
        'border-color': surface,
        'border-width': 2,
      },
    },
    ...kindStyles,
    {
      selector: 'edge',
      style: {
        label: 'data(relation)',
        width: 1.5,
        'line-color': muted,
        'target-arrow-color': muted,
        'target-arrow-shape': 'triangle',
        'curve-style': 'bezier',
        color: muted,
        'font-size': 9,
        'text-rotation': 'autorotate',
        'text-background-color': surface,
        'text-background-opacity': 0.9,
        'text-background-padding': '2px',
      },
    },
    {
      selector: 'node:selected, node.hovered',
      style: {
        'border-color': primary,
        'border-width': 4,
      },
    },
    {
      selector: 'edge:selected, edge.hovered',
      style: {
        width: 4,
        'line-color': primary,
        'target-arrow-color': primary,
        color: text,
        'font-weight': 700,
      },
    },
    {
      selector: '.dimmed',
      style: {
        opacity: 0.14,
      },
    },
  ];
}
