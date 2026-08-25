import { describe, expect, test } from 'vitest';

import type { GraphEdgeView } from '../data/types';
import { relationOptionsFor } from './relations';

const expectedOptions = [
  { value: 'all', label: 'All' },
  { value: 'depends-on', label: 'Depends-on' },
  { value: 'maps-to', label: 'Maps-to' },
  { value: 'refines', label: 'Refines' },
  { value: 'requires', label: 'Requires' },
  { value: 'excludes', label: 'Excludes' },
  { value: 'constrains', label: 'Constrains' },
  { value: 'verifies', label: 'Verifies' },
  { value: 'uses', label: 'Uses' },
  { value: 'implements', label: 'Implements' },
  { value: 'proves', label: 'Proves' },
];

describe('canonical graph relation options', () => {
  test('an empty graph still offers All followed by every strategic and tactical relation', () => {
    expect(relationOptionsFor([])).toEqual(expectedOptions);
  });

  test('a sparse graph does not hide relations absent from its edges', () => {
    const sparseEdges: GraphEdgeView[] = [
      {
        from: { kind: 'intent', id: 'INT-0042' },
        relation: 'requires',
        to: { kind: 'intent', id: 'INT-0017' },
      },
    ];

    expect(relationOptionsFor(sparseEdges)).toEqual(expectedOptions);
  });
});
