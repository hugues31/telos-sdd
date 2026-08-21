import { describe, expect, it } from 'vitest';

import { filterNotions, groupNotions, percentage } from './page-data';
import type { NotionView } from '../data/types';

const notions: NotionView[] = [
  { name: 'Customer', kind: 'entity', definition: 'Receives invoices.', canonical: 'notion Customer entity {}' },
  { name: 'InvoiceIssued', kind: 'event', definition: 'An invoice was issued.', canonical: 'notion InvoiceIssued event {}' },
];

describe('filterNotions', () => {
  it('matches notion fields case-insensitively and retains snapshot order', () => {
    expect(filterNotions(notions, 'INVOICE')).toEqual([notions[0], notions[1]]);
  });
});

describe('percentage', () => {
  it('returns zero for an empty denominator', () => {
    expect(percentage(0, 0)).toBe(0);
  });
});

describe('groupNotions', () => {
  it('uses a stable kind order while retaining snapshot order within a group', () => {
    expect(groupNotions([notions[1], notions[0]])).toEqual([
      { kind: 'entity', notions: [notions[0]] },
      { kind: 'event', notions: [notions[1]] },
    ]);
  });
});
