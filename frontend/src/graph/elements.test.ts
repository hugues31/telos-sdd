import { describe, expect, it } from 'vitest';

import type { GraphEdgeView, GraphNodeView } from '../data/types';
import { buildGraphElements, dimmedElementIds, graphSelectionId, nodeId } from './elements';
import type { VisibleGraphEdge } from './projection';

const context = { kind: 'context', id: 'billing' } as const;
const capability = { kind: 'capability', id: 'billing/settlement' } as const;

const nodes: GraphNodeView[] = [
  { key: context, label: 'Billing', parent: null },
  { key: capability, label: 'Settlement', parent: context },
  { key: { kind: 'notion', id: 'Invoice' }, label: 'A bill.', parent: context },
  {
    key: { kind: 'intent', id: 'INT-0042' },
    label: 'Settle invoices',
    parent: capability,
  },
  {
    key: { kind: 'scenario', id: 'SCN-0107' },
    label: 'Full payment',
    parent: capability,
  },
  {
    key: { kind: 'constraint', id: 'CON-0011' },
    label: 'No unwraps',
    parent: null,
  },
  {
    key: { kind: 'code', id: 'src/invoice.rs' },
    label: 'src/invoice.rs',
    parent: null,
  },
  {
    key: { kind: 'test', id: 'tests/invoice.rs::settles' },
    label: 'settles',
    parent: null,
  },
];

const semanticEdges: GraphEdgeView[] = [
  {
    from: { kind: 'intent', id: 'INT-0042' },
    relation: 'uses',
    to: { kind: 'notion', id: 'Invoice' },
  },
  {
    from: { kind: 'intent', id: 'INT-0042' },
    relation: 'requires',
    to: { kind: 'notion', id: 'Invoice' },
  },
  {
    from: { kind: 'scenario', id: 'SCN-0107' },
    relation: 'verifies',
    to: { kind: 'intent', id: 'INT-0042' },
  },
  {
    from: { kind: 'constraint', id: 'CON-0011' },
    relation: 'constrains',
    to: { kind: 'intent', id: 'INT-0042' },
  },
  {
    from: { kind: 'code', id: 'src/invoice.rs' },
    relation: 'implements',
    to: { kind: 'intent', id: 'INT-0042' },
  },
  {
    from: { kind: 'test', id: 'tests/invoice.rs::settles' },
    relation: 'proves',
    to: { kind: 'scenario', id: 'SCN-0107' },
  },
];

const edges: VisibleGraphEdge[] = semanticEdges.map((edge) => ({
  ...edge,
  relation: edge.relation as VisibleGraphEdge['relation'],
  members: [edge],
}));

describe('nodeId', () => {
  it('uses the canonical Rust dom_key format', () => {
    expect(nodeId({ kind: 'scenario', id: 'SCN-0107' })).toBe('scenario:SCN-0107');
  });

  it('derives the rendered id for retained node and edge selections', () => {
    expect(
      graphSelectionId({ type: 'node', entity: context, label: 'Billing' }),
    ).toBe('context:billing');
    expect(
      graphSelectionId({
        type: 'edge',
        relation: 'uses',
        source: semanticEdges[0].from,
        target: semanticEdges[0].to,
        members: [semanticEdges[0]],
      }),
    ).toBe('edge:intent%3AINT-0042:uses:notion%3AInvoice');
  });
});

describe('buildGraphElements', () => {
  it('emits contexts before capabilities and assigns compound parents', () => {
    const nodeElements = buildGraphElements(nodes, []).filter(
      (element) => element.group === 'nodes',
    );

    expect(nodeElements.map((element) => element.data.id)).toEqual([
      'context:billing',
      'capability:billing/settlement',
      'notion:Invoice',
      'intent:INT-0042',
      'scenario:SCN-0107',
      'constraint:CON-0011',
      'code:src/invoice.rs',
      'test:tests/invoice.rs::settles',
    ]);
    expect(nodeElements[0].data.parent).toBeUndefined();
    expect(nodeElements[0].data.container).toBe(true);
    expect(nodeElements[1].data.parent).toBe('context:billing');
    expect(nodeElements[1].data.container).toBe(true);
    expect(nodeElements[2].data.parent).toBe('context:billing');
    expect(nodeElements[2].data.container).toBeUndefined();
    expect(nodeElements[3].data.parent).toBe('capability:billing/settlement');
    expect(nodeElements[6].data.parent).toBeUndefined();
  });

  it('marks container labels as expanded or collapsed', () => {
    const nodeElements = buildGraphElements(
      nodes,
      [],
      new Set(['capability:billing/settlement']),
    ).filter((element) => element.group === 'nodes');

    expect(nodeElements[0].data.label).toBe('− Billing');
    expect(nodeElements[0].data.collapsed).toBeUndefined();
    expect(nodeElements[1].data.label).toBe('+ Settlement');
    expect(nodeElements[1].data.collapsed).toBe(true);
    expect(nodeElements[2].data.label).toBe('A bill.');
    expect(nodeElements[2].data.collapsed).toBeUndefined();
  });

  it('caps long rendered labels without losing the full selection label', () => {
    const longLabel = '1234567890123456789012345678901234567890EXTRA';
    const element = buildGraphElements(
      [
        { key: context, label: 'Billing', parent: null },
        { key: capability, label: 'Settlement', parent: context },
        {
          key: { kind: 'intent', id: 'INT-0042' },
          label: longLabel,
          parent: capability,
        },
      ],
      [],
    ).find((candidate) => candidate.data.id === 'intent:INT-0042');

    expect(element?.data.rawLabel).toBe(longLabel);
    expect(element?.data.label).toBe('123456789012345678901234567890123456789…');
  });

  it('preserves the members and count of an aggregated visible edge', () => {
    const duplicate = { ...semanticEdges[0] };
    const aggregated: VisibleGraphEdge = {
      ...edges[0],
      members: [semanticEdges[0], duplicate],
    };
    const edgeElement = buildGraphElements(nodes, [aggregated]).find(
      (element) => element.group === 'edges',
    );

    expect(edgeElement?.data).toEqual({
      id: 'edge:intent%3AINT-0042:uses:notion%3AInvoice',
      source: 'intent:INT-0042',
      target: 'notion:Invoice',
      relation: 'uses',
      sourceKind: 'intent',
      sourceId: 'INT-0042',
      targetKind: 'notion',
      targetId: 'Invoice',
      count: 2,
      displayLabel: 'uses ×2',
      members: [semanticEdges[0], duplicate],
    });
  });

  it('preserves every visible relation value', () => {
    const relationData = buildGraphElements(nodes, edges)
      .filter((element) => element.group === 'edges')
      .map((element) => element.data.relation);

    expect(relationData).toEqual([
      'uses',
      'requires',
      'verifies',
      'constrains',
      'implements',
      'proves',
    ]);
  });
});

describe('dimmedElementIds', () => {
  it('dims nothing for the all filter', () => {
    expect([...dimmedElementIds(buildGraphElements(nodes, edges), 'all')]).toEqual([]);
  });

  it('keeps compound boundaries visible while dimming unrelated ordinary nodes', () => {
    const dimmed = dimmedElementIds(buildGraphElements(nodes, edges), 'proves');

    expect(dimmed.has('context:billing')).toBe(false);
    expect(dimmed.has('capability:billing/settlement')).toBe(false);
    expect(dimmed.has('notion:Invoice')).toBe(true);
    expect(dimmed.has('test:tests/invoice.rs::settles')).toBe(false);
  });
});
