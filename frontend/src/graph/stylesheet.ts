import type cytoscape from 'cytoscape';

import type { GraphKeyKind } from '../data/types';

const KIND_SHAPES: Record<GraphKeyKind, cytoscape.Css.NodeShape> = {
  context: 'round-rectangle',
  capability: 'barrel',
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
        color: text,
        'font-size': 10,
        'font-weight': 600,
        'text-wrap': 'ellipsis',
        'text-max-width': '120px',
        'text-overflow-wrap': 'anywhere',
        'border-color': surface,
        'border-width': 2,
      },
    },
    {
      selector: 'node:childless',
      style: {
        width: 44,
        height: 44,
        'text-valign': 'bottom',
        'text-margin-y': 8,
        'text-background-color': surface,
        'text-background-opacity': 0.9,
        'text-background-padding': '2px',
      },
    },
    ...kindStyles,
    {
      selector: 'node[container]',
      style: {
        shape: 'round-rectangle',
        width: 112,
        height: 44,
        padding: '24px',
        'background-opacity': 0.06,
        'text-valign': 'top',
        'text-halign': 'left',
        'text-margin-x': 8,
        'text-margin-y': 8,
        'font-size': 12,
        'font-weight': 700,
        'text-background-opacity': 0,
      },
    },
    {
      selector: 'node[kind = "context"]',
      style: {
        'border-color': cssToken(tokens, '--k-context'),
        'border-width': 3,
      },
    },
    {
      selector: 'node[kind = "capability"]',
      style: {
        'border-color': cssToken(tokens, '--k-capability'),
        'border-width': 2,
        'border-style': 'dashed',
      },
    },
    {
      selector: 'node[collapsed]',
      style: {
        width: 112,
        height: 36,
        padding: '0px',
        'background-opacity': 0.12,
        'text-valign': 'center',
        'text-halign': 'center',
        'text-margin-x': 0,
        'text-margin-y': 0,
      },
    },
    {
      selector: 'edge',
      style: {
        label: 'data(displayLabel)',
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
