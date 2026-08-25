import { describe, expect, test } from 'vitest';

import type { GraphNodeView } from '../data/types';
import { searchEntities, shortcutLabel, shouldOpenGlobalSearch } from './entities';

const nodes: GraphNodeView[] = [
  {
    key: { kind: 'intent', id: 'INT-0011' },
    label: 'Verify adult eligibility',
    parent: { kind: 'capability', id: 'travel/booking' },
  },
  {
    key: { kind: 'scenario', id: 'SCN-0016' },
    label: 'Traveller who is adult is accepted',
    parent: { kind: 'capability', id: 'travel/booking' },
  },
  {
    key: { kind: 'notion', id: 'Payment' },
    label: 'A settled payment',
    parent: { kind: 'context', id: 'billing' },
  },
  {
    key: { kind: 'intent', id: 'INT-0099' },
    label: 'Payment clears the balance',
    parent: { kind: 'capability', id: 'billing/settlement' },
  },
];

function keyEvent(
  key: string,
  options: { ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; target?: unknown } = {},
): KeyboardEvent {
  return {
    key,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    target: { tagName: 'BODY' },
    ...options,
  } as KeyboardEvent;
}

describe('entity search', () => {
  test('ranks exact, prefix, label substring, and kind matches', () => {
    expect(searchEntities(nodes, 'INT-0011').map((node) => node.key.id)).toEqual(['INT-0011']);
    expect(searchEntities(nodes, 'INT').map((node) => node.key.id)).toEqual([
      'INT-0011',
      'INT-0099',
    ]);
    expect(searchEntities(nodes, 'adult').map((node) => node.key.id)).toEqual([
      'INT-0011',
      'SCN-0016',
    ]);
    expect(searchEntities(nodes, 'scenario').map((node) => node.key.id)).toEqual(['SCN-0016']);
  });

  test('preserves snapshot order for equal-rank matches', () => {
    expect(searchEntities(nodes, 'payment').map((node) => node.key.id)).toEqual([
      'Payment',
      'INT-0099',
    ]);
  });

  test('ignores whitespace-only queries and caps results at eight by default', () => {
    const tenIntentNodes = Array.from({ length: 10 }, (_, index): GraphNodeView => ({
      key: { kind: 'intent', id: `INT-${index}` },
      label: `Intent ${index}`,
      parent: { kind: 'capability', id: 'test/search' },
    }));

    expect(searchEntities(nodes, '   ')).toEqual([]);
    expect(searchEntities(tenIntentNodes, 'intent')).toHaveLength(8);
    expect(searchEntities(tenIntentNodes, 'intent', 3)).toHaveLength(3);
  });
});

describe('global search shortcuts', () => {
  test('recognizes Command/Ctrl K from any target', () => {
    expect(shouldOpenGlobalSearch(keyEvent('k', { metaKey: true, target: { tagName: 'INPUT' } })))
      .toBe(true);
    expect(shouldOpenGlobalSearch(keyEvent('K', { ctrlKey: true, target: { tagName: 'A' } })))
      .toBe(true);
  });

  test('recognizes slash on the page but not from interactive or editable targets', () => {
    expect(shouldOpenGlobalSearch(keyEvent('/'))).toBe(true);

    for (const tagName of ['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON', 'A']) {
      expect(shouldOpenGlobalSearch(keyEvent('/', { target: { tagName } }))).toBe(false);
    }
    expect(
      shouldOpenGlobalSearch(
        keyEvent('/', { target: { tagName: 'DIV', isContentEditable: true } }),
      ),
    ).toBe(false);
  });

  test('does not capture modified slash or unrelated keys', () => {
    expect(shouldOpenGlobalSearch(keyEvent('/', { ctrlKey: true }))).toBe(false);
    expect(shouldOpenGlobalSearch(keyEvent('f'))).toBe(false);
  });

  test('formats the platform-specific shortcut hint', () => {
    expect(shortcutLabel('MacIntel')).toBe('⌘K');
    expect(shortcutLabel('Win32')).toBe('Ctrl K');
    expect(shortcutLabel('Linux x86_64')).toBe('Ctrl K');
  });
});
