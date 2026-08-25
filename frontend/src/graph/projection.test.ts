import { describe, expect, it } from 'vitest';

import type { GraphEdgeView, GraphNodeView } from '../data/types';
import { graphKeyId, projectVisibleGraph } from './projection';

const contextPet = { kind: 'context', id: 'pet' } as const;
const lifecycle = { kind: 'capability', id: 'pet/lifecycle' } as const;
const pet = { kind: 'notion', id: 'pet/Pet' } as const;
const hatch = { kind: 'intent', id: 'INT-0001' } as const;
const hatches = { kind: 'scenario', id: 'SCN-0001' } as const;
const contextTerminal = { kind: 'context', id: 'terminal' } as const;
const portrait = { kind: 'capability', id: 'terminal/portrait' } as const;
const petView = { kind: 'notion', id: 'terminal/PetView' } as const;
const test = { kind: 'test', id: 'tests/test_hatch.py::scn_0001' } as const;

const nodes: GraphNodeView[] = [
  { key: contextPet, label: 'Pet', parent: null },
  { key: lifecycle, label: 'Lifecycle', parent: contextPet },
  { key: pet, label: 'Pet aggregate', parent: contextPet },
  { key: hatch, label: 'Hatch', parent: lifecycle },
  { key: hatches, label: 'An egg hatches', parent: lifecycle },
  { key: contextTerminal, label: 'Terminal', parent: null },
  { key: portrait, label: 'Portrait', parent: contextTerminal },
  { key: petView, label: 'Terminal pet view', parent: portrait },
  { key: test, label: 'Hatch test', parent: null },
];

const edges: GraphEdgeView[] = [
  { from: lifecycle, relation: 'belongs-to', to: contextPet },
  { from: hatch, relation: 'belongs-to', to: lifecycle },
  { from: hatch, relation: 'uses', to: pet },
  { from: hatches, relation: 'verifies', to: hatch },
  { from: hatches, relation: 'uses', to: pet },
  { from: test, relation: 'proves', to: hatches },
  { from: pet, relation: 'maps-to', to: petView },
  { from: contextTerminal, relation: 'depends-on', to: contextPet },
];

describe('projectVisibleGraph', () => {
  it('uses compound containment instead of rendering belongs-to edges', () => {
    const visible = projectVisibleGraph(nodes, edges, new Set());

    expect(visible.nodes.map((node) => graphKeyId(node.key))).toEqual([
      'capability:pet/lifecycle',
      'capability:terminal/portrait',
      'context:pet',
      'context:terminal',
      'intent:INT-0001',
      'notion:pet/Pet',
      'notion:terminal/PetView',
      'scenario:SCN-0001',
      'test:tests/test_hatch.py::scn_0001',
    ]);
    expect(visible.edges.map((edge) => edge.relation)).toEqual([
      'depends-on',
      'uses',
      'maps-to',
      'uses',
      'verifies',
      'proves',
    ]);
  });

  it('sorts visible nodes and aggregate edges by stable rendered ids', () => {
    const visible = projectVisibleGraph([...nodes].reverse(), [...edges].reverse(), new Set());
    const nodeIds = visible.nodes.map((node) => graphKeyId(node.key));
    const edgeIds = visible.edges.map(
      (edge) => `${graphKeyId(edge.from)}:${edge.relation}:${graphKeyId(edge.to)}`,
    );

    expect(nodeIds).toEqual([...nodeIds].sort());
    expect(edgeIds).toEqual([...edgeIds].sort());
  });

  it('redirects hidden endpoints and losslessly aggregates equal visible relations', () => {
    const visible = projectVisibleGraph(nodes, edges, new Set(['capability:pet/lifecycle']));

    expect(visible.nodes.map((node) => graphKeyId(node.key))).not.toContain('intent:INT-0001');
    expect(visible.nodes.map((node) => graphKeyId(node.key))).not.toContain('scenario:SCN-0001');

    const uses = visible.edges.find(
      (edge) =>
        graphKeyId(edge.from) === 'capability:pet/lifecycle' &&
        edge.relation === 'uses' &&
        graphKeyId(edge.to) === 'notion:pet/Pet',
    );
    expect(uses?.members).toEqual([edges[2], edges[4]]);
    expect(
      visible.edges.some(
        (edge) =>
          graphKeyId(edge.from) === 'capability:pet/lifecycle' &&
          edge.relation === 'verifies' &&
          graphKeyId(edge.to) === 'capability:pet/lifecycle',
      ),
    ).toBe(false);
    expect(visible.edges).toContainEqual({
      from: test,
      relation: 'proves',
      to: lifecycle,
      members: [edges[5]],
    });
  });

  it('redirects every hidden descendant to a collapsed context', () => {
    const visible = projectVisibleGraph(nodes, edges, new Set(['context:pet']));

    expect(visible.nodes.map((node) => graphKeyId(node.key))).toEqual([
      'capability:terminal/portrait',
      'context:pet',
      'context:terminal',
      'notion:terminal/PetView',
      'test:tests/test_hatch.py::scn_0001',
    ]);
    expect(visible.edges).toContainEqual({
      from: contextPet,
      relation: 'maps-to',
      to: petView,
      members: [edges[6]],
    });
    expect(visible.edges).toContainEqual({
      from: test,
      relation: 'proves',
      to: contextPet,
      members: [edges[5]],
    });
  });

  it('rejects a capability nested below another capability', () => {
    const nested: GraphNodeView[] = [
      { key: contextPet, label: 'Pet', parent: null },
      { key: lifecycle, label: 'Lifecycle', parent: contextPet },
      {
        key: { kind: 'capability', id: 'pet/lifecycle/growth' },
        label: 'Growth',
        parent: lifecycle,
      },
    ];

    expect(() => projectVisibleGraph(nested, [], new Set())).toThrow(
      'Capability parent must be a context: capability:pet/lifecycle/growth',
    );
  });

  it('rejects a missing parent container', () => {
    expect(() =>
      projectVisibleGraph(
        [
          {
            key: hatch,
            label: 'Hatch',
            parent: { kind: 'capability', id: 'pet/missing' },
          },
        ],
        [],
        new Set(),
      ),
    ).toThrow('Missing graph parent: capability:pet/missing');
  });

  it('rejects a parent cycle', () => {
    const cyclic: GraphNodeView[] = [
      {
        key: { kind: 'capability', id: 'pet/one' },
        label: 'One',
        parent: { kind: 'context', id: 'pet' },
      },
      {
        key: contextPet,
        label: 'Pet',
        parent: { kind: 'context', id: 'pet' },
      },
    ];

    expect(() => projectVisibleGraph(cyclic, [], new Set())).toThrow(
      'Root graph node cannot have a parent: context:pet',
    );
  });
});
