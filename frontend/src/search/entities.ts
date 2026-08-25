import type { GraphNodeView } from '../data/types';

interface RankedEntity {
  node: GraphNodeView;
  index: number;
  rank: number;
}

function matchRank(node: GraphNodeView, needle: string): number | null {
  const id = node.key.id.toLocaleLowerCase();
  const label = node.label.toLocaleLowerCase();
  const kind = node.key.kind.toLocaleLowerCase();

  if (id === needle || label === needle) return 0;
  if (id.startsWith(needle) || label.startsWith(needle)) return 1;
  if (id.includes(needle) || label.includes(needle)) return 2;
  if (kind.includes(needle)) return 3;
  return null;
}

export function searchEntities(
  nodes: GraphNodeView[],
  query: string,
  limit = 8,
): GraphNodeView[] {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return [];

  return nodes
    .map((node, index) => ({ node, index, rank: matchRank(node, needle) }))
    .filter((entry): entry is RankedEntity => entry.rank !== null)
    .sort((left, right) => left.rank - right.rank || left.index - right.index)
    .slice(0, Math.max(0, limit))
    .map((entry) => entry.node);
}

interface KeyboardTarget {
  tagName?: unknown;
  isContentEditable?: unknown;
  closest?: unknown;
}

function isInteractiveTarget(rawTarget: EventTarget | null): boolean {
  const target = rawTarget as KeyboardTarget | null;
  if (!target) return false;

  const tagName = typeof target.tagName === 'string' ? target.tagName.toLocaleLowerCase() : '';
  if (['input', 'textarea', 'select', 'button', 'a'].includes(tagName)) return true;
  if (target.isContentEditable === true) return true;

  if (typeof target.closest === 'function') {
    const closest = target.closest as (selector: string) => unknown;
    return Boolean(
      closest.call(
        target,
        'input, textarea, select, button, a, [contenteditable="true"], [contenteditable="plaintext-only"]',
      ),
    );
  }

  return false;
}

export function shouldOpenGlobalSearch(event: KeyboardEvent): boolean {
  const key = event.key.toLocaleLowerCase();
  if (key === 'k' && (event.metaKey || event.ctrlKey) && !event.altKey) return true;

  return (
    event.key === '/' &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    !isInteractiveTarget(event.target)
  );
}

export function shortcutLabel(platform: string): string {
  return /mac|iphone|ipad|ipod/i.test(platform) ? '⌘K' : 'Ctrl K';
}
