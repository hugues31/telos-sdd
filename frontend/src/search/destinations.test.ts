import { describe, expect, test } from 'vitest';

import type { GraphKey } from '../data/types';
import { entityDestination } from './destinations';

describe('entity destinations', () => {
  test.each<[GraphKey, string | undefined, unknown]>([
    [
      { kind: 'context', id: 'billing' },
      undefined,
      { name: 'contexts', hash: '#context-billing' },
    ],
    [
      { kind: 'capability', id: 'billing/invoicing' },
      undefined,
      { name: 'contexts', hash: '#capability-billing-invoicing' },
    ],
    [
      { kind: 'intent', id: 'INT-0011' },
      undefined,
      { name: 'intent-detail', params: { id: 'INT-0011' } },
    ],
    [
      { kind: 'scenario', id: 'SCN-0016' },
      'INT-0011',
      { name: 'intent-detail', params: { id: 'INT-0011' }, hash: '#scenario-SCN-0016' },
    ],
    [
      { kind: 'notion', id: 'Customer' },
      undefined,
      { name: 'glossary', hash: '#notion-Customer' },
    ],
    [
      { kind: 'constraint', id: 'CON-0003' },
      undefined,
      { name: 'coverage', hash: '#constraint-CON-0003' },
    ],
    [
      { kind: 'code', id: 'src/pet.ts' },
      undefined,
      { name: 'graph', query: { focusKind: 'code', focusId: 'src/pet.ts' } },
    ],
    [
      { kind: 'test', id: 'tests/pet.test.ts' },
      undefined,
      { name: 'graph', query: { focusKind: 'test', focusId: 'tests/pet.test.ts' } },
    ],
  ])('maps %s to its natural route', (key, scenarioParent, expected) => {
    expect(entityDestination(key, scenarioParent)).toEqual(expected);
  });

  test('does not invent a destination for an orphan scenario', () => {
    expect(entityDestination({ kind: 'scenario', id: 'SCN-404' })).toBeNull();
  });
});
