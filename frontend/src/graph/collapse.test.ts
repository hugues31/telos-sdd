import { describe, expect, it } from 'vitest';

import type { GraphNodeView } from '../data/types';
import {
  COLLAPSE_STORAGE_KEY,
  collapsedIdsSignature,
  containerNodeIds,
  loadCollapsedIds,
  normalizeCollapsedIds,
  safeSessionStorage,
  storeCollapsedIds,
  toggleCollapsedId,
} from './collapse';

const context = { kind: 'context', id: 'billing' } as const;
const capability = { kind: 'capability', id: 'billing/settlement' } as const;
const nodes: GraphNodeView[] = [
  { key: context, label: 'Billing', parent: null },
  { key: capability, label: 'Settlement', parent: context },
  {
    key: { kind: 'intent', id: 'INT-0042' },
    label: 'Settle invoices',
    parent: capability,
  },
];

class MemoryStorage implements Storage {
  readonly values = new Map<string, string>();

  get length(): number {
    return this.values.size;
  }

  clear(): void {
    this.values.clear();
  }

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  key(index: number): string | null {
    return [...this.values.keys()][index] ?? null;
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

describe('compound graph collapse state', () => {
  it('gives equivalent collapse sets a stable render signature', () => {
    expect(
      collapsedIdsSignature(['context:billing', 'capability:billing/settlement']),
    ).toBe(collapsedIdsSignature(['capability:billing/settlement', 'context:billing']));
  });

  it('returns every context and capability for collapse all', () => {
    expect(containerNodeIds(nodes)).toEqual([
      'context:billing',
      'capability:billing/settlement',
    ]);
  });

  it('prunes missing and non-container ids from restored state', () => {
    expect(
      [...normalizeCollapsedIds(nodes, ['intent:INT-0042', 'context:missing', 'context:billing'])],
    ).toEqual(['context:billing']);
  });

  it('expanding a context preserves the remembered capability state', () => {
    const collapsed = new Set(['context:billing', 'capability:billing/settlement']);

    expect([...toggleCollapsedId(collapsed, 'context:billing')]).toEqual([
      'capability:billing/settlement',
    ]);
  });

  it('round-trips sorted collapse state through session storage', () => {
    const storage = new MemoryStorage();
    storeCollapsedIds(
      storage,
      new Set(['context:billing', 'capability:billing/settlement']),
    );

    expect(storage.getItem(COLLAPSE_STORAGE_KEY)).toBe(
      '["capability:billing/settlement","context:billing"]',
    );
    expect([...loadCollapsedIds(storage, nodes)]).toEqual([
      'capability:billing/settlement',
      'context:billing',
    ]);
  });

  it('fails closed to expanded state for malformed session data', () => {
    const storage = new MemoryStorage();
    storage.setItem(COLLAPSE_STORAGE_KEY, '{bad json');

    expect([...loadCollapsedIds(storage, nodes)]).toEqual([]);
  });

  it('falls back to in-memory state when the browser denies session storage access', () => {
    const denied = {
      get sessionStorage(): Storage {
        throw new DOMException('Access denied', 'SecurityError');
      },
    };

    expect(safeSessionStorage(denied)).toBeNull();
  });
});
