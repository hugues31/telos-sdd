import type { GraphKey, GraphKeyKind, GraphNodeView } from '../data/types';
import { graphKeyId } from './projection';

export interface GraphFocusRequest {
  key: GraphKey;
  token: number;
}

const GRAPH_KEY_KINDS = new Set<GraphKeyKind>([
  'context',
  'capability',
  'notion',
  'intent',
  'scenario',
  'constraint',
  'code',
  'test',
]);

export function expandAncestorsForNode(
  nodes: GraphNodeView[],
  collapsedIds: ReadonlySet<string>,
  key: GraphKey,
): Set<string> {
  const next = new Set(collapsedIds);
  const nodesById = new Map(nodes.map((node) => [graphKeyId(node.key), node]));
  let current = nodesById.get(graphKeyId(key));

  while (current?.parent) {
    const parentId = graphKeyId(current.parent);
    next.delete(parentId);
    current = nodesById.get(parentId);
  }

  return next;
}

export function graphFocusFromQuery(query: Record<string, unknown>): GraphKey | null {
  const kind = query.focusKind;
  const id = query.focusId;

  if (typeof kind !== 'string' || !GRAPH_KEY_KINDS.has(kind as GraphKeyKind)) return null;
  if (typeof id !== 'string' || id.trim().length === 0) return null;
  return { kind: kind as GraphKeyKind, id };
}
