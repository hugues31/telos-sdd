import { describe, expect, test } from 'vitest';

import type { GraphKey } from '../data/types';
import {
  consumerRows,
  filterConsumerRows,
  sortConsumerRows,
  type ConsumerRow,
} from './glossary-consumers';

describe('glossary consumer rows', () => {
  const consumers: GraphKey[] = [
    { kind: 'scenario', id: 'SCN-0016' },
    { kind: 'intent', id: 'INT-0011' },
  ];
  const rows = consumerRows(
    consumers,
    new Map([['INT-0011', { title: 'Adults are harder to please' }]]),
    new Map([['SCN-0016', { title: 'the same game, a smaller joy' }]]),
  );

  test('resolves intent and scenario titles while preserving entity types', () => {
    expect(rows).toEqual([
      {
        kind: 'scenario',
        id: 'SCN-0016',
        title: 'the same game, a smaller joy',
        entity: consumers[0],
      },
      {
        kind: 'intent',
        id: 'INT-0011',
        title: 'Adults are harder to please',
        entity: consumers[1],
      },
    ]);
  });

  test('sorts copies by the requested column with ID as a stable tiebreaker', () => {
    expect(sortConsumerRows(rows, 'kind', 'asc').map((row) => row.id)).toEqual([
      'INT-0011',
      'SCN-0016',
    ]);

    const tied: ConsumerRow[] = [
      { kind: 'scenario', id: 'SCN-2', title: 'Same', entity: { kind: 'scenario', id: 'SCN-2' } },
      { kind: 'scenario', id: 'SCN-1', title: 'Same', entity: { kind: 'scenario', id: 'SCN-1' } },
    ];
    expect(sortConsumerRows(tied, 'title', 'asc').map((row) => row.id)).toEqual([
      'SCN-1',
      'SCN-2',
    ]);
    expect(tied.map((row) => row.id)).toEqual(['SCN-2', 'SCN-1']);
  });

  test('filters by precise consumer kind without mutating the source', () => {
    expect(filterConsumerRows(rows, 'scenario')).toEqual([rows[0]]);
    expect(filterConsumerRows(rows, '')).toEqual(rows);
    expect(rows.map((row) => row.id)).toEqual(['SCN-0016', 'INT-0011']);
  });

  test('ignores unrelated graph kinds and falls back to the reference ID', () => {
    const input: GraphKey[] = [
      { kind: 'code', id: 'src/pet.ts' },
      { kind: 'intent', id: 'INT-404' },
      { kind: 'scenario', id: 'SCN-404' },
    ];

    expect(consumerRows(input, new Map(), new Map()).map((row) => row.title)).toEqual([
      'INT-404',
      'SCN-404',
    ]);
    expect(input).toHaveLength(3);
  });
});
