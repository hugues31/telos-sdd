import { describe, expect, it } from 'vitest';

import type { GraphEdgeView, GraphNodeView } from '../data/types';
import { buildGraphElements, dimmedElementIds, nodeId } from './elements';

const nodes: GraphNodeView[] = [
  { key: { kind: 'notion', id: 'Invoice' }, label: 'A bill.' },
  { key: { kind: 'intent', id: 'INT-0042' }, label: 'Settle invoices' },
  { key: { kind: 'scenario', id: 'SCN-0107' }, label: 'Full payment' },
  { key: { kind: 'constraint', id: 'CON-0011' }, label: 'No unwraps' },
  { key: { kind: 'code', id: 'src/invoice.rs' }, label: 'src/invoice.rs' },
  { key: { kind: 'test', id: 'tests/invoice.rs::settles' }, label: 'settles' },
];

const edges: GraphEdgeView[] = [
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
  {
    from: { kind: 'intent', id: 'INT-0042' },
    relation: 'refines',
    to: { kind: 'intent', id: 'INT-0001' },
  },
  {
    from: { kind: 'intent', id: 'INT-0001' },
    relation: 'excludes',
    to: { kind: 'intent', id: 'INT-0042' },
  },
];

describe('nodeId', () => {
  it('uses the canonical Rust dom_key format', () => {
    expect(nodeId({ kind: 'scenario', id: 'SCN-0107' })).toBe('scenario:SCN-0107');
  });
});

describe('buildGraphElements', () => {
  it('maps all six node kinds and preserves selection data', () => {
    const elements = buildGraphElements(nodes, []);

    expect(elements).toEqual([
      {
        group: 'nodes',
        data: { id: 'notion:Invoice', kind: 'notion', entityId: 'Invoice', label: 'A bill.' },
      },
      {
        group: 'nodes',
        data: {
          id: 'intent:INT-0042',
          kind: 'intent',
          entityId: 'INT-0042',
          label: 'Settle invoices',
        },
      },
      {
        group: 'nodes',
        data: {
          id: 'scenario:SCN-0107',
          kind: 'scenario',
          entityId: 'SCN-0107',
          label: 'Full payment',
        },
      },
      {
        group: 'nodes',
        data: {
          id: 'constraint:CON-0011',
          kind: 'constraint',
          entityId: 'CON-0011',
          label: 'No unwraps',
        },
      },
      {
        group: 'nodes',
        data: {
          id: 'code:src/invoice.rs',
          kind: 'code',
          entityId: 'src/invoice.rs',
          label: 'src/invoice.rs',
        },
      },
      {
        group: 'nodes',
        data: {
          id: 'test:tests/invoice.rs::settles',
          kind: 'test',
          entityId: 'tests/invoice.rs::settles',
          label: 'settles',
        },
      },
    ]);
  });

  it('gives parallel relations stable unique ids and preserves both endpoints', () => {
    const elements = buildGraphElements(nodes.slice(0, 2), edges.slice(0, 2));

    expect(elements.slice(2)).toEqual([
      {
        group: 'edges',
        data: {
          id: 'edge:intent%3AINT-0042:uses:notion%3AInvoice',
          source: 'intent:INT-0042',
          target: 'notion:Invoice',
          relation: 'uses',
          sourceKind: 'intent',
          sourceId: 'INT-0042',
          targetKind: 'notion',
          targetId: 'Invoice',
        },
      },
      {
        group: 'edges',
        data: {
          id: 'edge:intent%3AINT-0042:requires:notion%3AInvoice',
          source: 'intent:INT-0042',
          target: 'notion:Invoice',
          relation: 'requires',
          sourceKind: 'intent',
          sourceId: 'INT-0042',
          targetKind: 'notion',
          targetId: 'Invoice',
        },
      },
    ]);
  });

  it('adds deterministic occurrence ordinals to identical parallel edges', () => {
    const duplicate = edges[0];
    const elements = buildGraphElements(nodes.slice(0, 2), [duplicate, duplicate]);

    expect(elements.slice(2).map((element) => element.data.id)).toEqual([
      'edge:intent%3AINT-0042:uses:notion%3AInvoice:0',
      'edge:intent%3AINT-0042:uses:notion%3AInvoice:1',
    ]);
  });

  it('preserves every graph relation value', () => {
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
      'refines',
      'excludes',
    ]);
  });
});

describe('dimmedElementIds', () => {
  it('dims nothing for the all filter', () => {
    expect([...dimmedElementIds(buildGraphElements(nodes, edges), 'all')]).toEqual([]);
  });

  it('dims unrelated edges and nodes for a precise relation', () => {
    const dimmed = dimmedElementIds(buildGraphElements(nodes, edges), 'proves');

    expect([...dimmed]).toEqual([
      'notion:Invoice',
      'intent:INT-0042',
      'constraint:CON-0011',
      'code:src/invoice.rs',
      'edge:intent%3AINT-0042:uses:notion%3AInvoice',
      'edge:intent%3AINT-0042:requires:notion%3AInvoice',
      'edge:scenario%3ASCN-0107:verifies:intent%3AINT-0042',
      'edge:constraint%3ACON-0011:constrains:intent%3AINT-0042',
      'edge:code%3Asrc%2Finvoice.rs:implements:intent%3AINT-0042',
      'edge:intent%3AINT-0042:refines:intent%3AINT-0001',
      'edge:intent%3AINT-0001:excludes:intent%3AINT-0042',
    ]);
  });
});
