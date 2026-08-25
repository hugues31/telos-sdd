import { describe, expect, test } from 'vitest';

import type { GraphNodeView } from '../data/types';
import { expandAncestorsForNode, graphFocusFromQuery } from './search';

const nodes: GraphNodeView[] = [
  { key: { kind: 'context', id: 'pet' }, label: 'Pet', parent: null },
  {
    key: { kind: 'capability', id: 'pet/care' },
    label: 'Care',
    parent: { kind: 'context', id: 'pet' },
  },
  {
    key: { kind: 'intent', id: 'INT-0011' },
    label: 'Care for a pet',
    parent: { kind: 'capability', id: 'pet/care' },
  },
  { key: { kind: 'code', id: 'src/pet.ts' }, label: 'src/pet.ts', parent: null },
];

describe('graph finder helpers', () => {
  test('expands every collapsed container ancestor of a chosen node', () => {
    expect(
      expandAncestorsForNode(
        nodes,
        new Set(['context:pet', 'capability:pet/care']),
        { kind: 'intent', id: 'INT-0011' },
      ),
    ).toEqual(new Set());
  });

  test('preserves unrelated collapsed containers and handles missing nodes', () => {
    const collapsed = new Set(['context:pet', 'context:billing']);
    expect(
      expandAncestorsForNode(nodes, collapsed, { kind: 'capability', id: 'pet/care' }),
    ).toEqual(new Set(['context:billing']));
    expect(expandAncestorsForNode(nodes, collapsed, { kind: 'intent', id: 'missing' })).toEqual(
      collapsed,
    );
  });

  test('parses only complete scalar focus queries with known graph kinds', () => {
    expect(graphFocusFromQuery({ focusKind: 'code', focusId: 'src/pet.ts' })).toEqual({
      kind: 'code',
      id: 'src/pet.ts',
    });
    expect(graphFocusFromQuery({ focusKind: 'unknown', focusId: 'x' })).toBeNull();
    expect(graphFocusFromQuery({ focusKind: 'intent' })).toBeNull();
    expect(graphFocusFromQuery({ focusKind: ['intent'], focusId: 'INT-0011' })).toBeNull();
    expect(graphFocusFromQuery({ focusKind: 'intent', focusId: '' })).toBeNull();
  });
});
