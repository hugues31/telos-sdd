import { describe, expect, it } from 'vitest';

import { replacementNodePositions } from './replacement';

describe('replacementNodePositions', () => {
  it('preserves retained positions and places new nodes beside their extent', () => {
    const previous = new Map([
      ['removed', { x: 900, y: 900 }],
      ['retained-a', { x: 10, y: 20 }],
      ['retained-b', { x: 50, y: 80 }],
    ]);

    expect([
      ...replacementNodePositions(
        ['retained-a', 'new-a', 'retained-b', 'new-b', 'new-c', 'new-d', 'new-e'],
        previous,
      ),
    ]).toEqual([
      ['retained-a', { x: 10, y: 20 }],
      ['retained-b', { x: 50, y: 80 }],
      ['new-a', { x: 146, y: 20 }],
      ['new-b', { x: 242, y: 20 }],
      ['new-c', { x: 338, y: 20 }],
      ['new-d', { x: 434, y: 20 }],
      ['new-e', { x: 146, y: 116 }],
    ]);
  });

  it('places an all-new replacement on a deterministic grid', () => {
    expect([
      ...replacementNodePositions(['new-a', 'new-b', 'new-c', 'new-d', 'new-e'], new Map()),
    ]).toEqual([
      ['new-a', { x: 0, y: 0 }],
      ['new-b', { x: 96, y: 0 }],
      ['new-c', { x: 192, y: 0 }],
      ['new-d', { x: 288, y: 0 }],
      ['new-e', { x: 0, y: 96 }],
    ]);
  });
});
