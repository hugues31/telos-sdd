import type { GraphNodeView } from '../data/types';
import { graphKeyId } from './projection';

export const COLLAPSE_STORAGE_KEY = 'telos.graph.collapsed.v1';

export function collapsedIdsSignature(ids: Iterable<string>): string {
  return JSON.stringify([...ids].sort());
}

export function safeSessionStorage(source: { readonly sessionStorage: Storage }): Storage | null {
  try {
    return source.sessionStorage;
  } catch {
    return null;
  }
}

function isContainer(node: GraphNodeView): boolean {
  return node.key.kind === 'context' || node.key.kind === 'capability';
}

export function containerNodeIds(nodes: GraphNodeView[]): string[] {
  return nodes.filter(isContainer).map((node) => graphKeyId(node.key));
}

export function normalizeCollapsedIds(
  nodes: GraphNodeView[],
  storedIds: Iterable<string>,
): Set<string> {
  const allowedIds = new Set(containerNodeIds(nodes));
  return new Set([...storedIds].filter((id) => allowedIds.has(id)));
}

export function toggleCollapsedId(collapsedIds: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(collapsedIds);
  if (next.has(id)) {
    next.delete(id);
  } else {
    next.add(id);
  }
  return next;
}

export function storeCollapsedIds(storage: Storage, collapsedIds: ReadonlySet<string>): void {
  try {
    storage.setItem(COLLAPSE_STORAGE_KEY, JSON.stringify([...collapsedIds].sort()));
  } catch {
    // Storage can be unavailable in hardened browser contexts. Collapse remains functional in memory.
  }
}

export function loadCollapsedIds(storage: Storage, nodes: GraphNodeView[]): Set<string> {
  try {
    const raw = storage.getItem(COLLAPSE_STORAGE_KEY);
    if (raw === null) return new Set();

    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed) || !parsed.every((id): id is string => typeof id === 'string')) {
      return new Set();
    }
    return normalizeCollapsedIds(nodes, parsed);
  } catch {
    return new Set();
  }
}
