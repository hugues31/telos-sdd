import type {
  GraphEdgeView,
  GraphKey,
  GraphNodeView,
  GraphRelation,
} from '../data/types';

export type VisibleGraphRelation = Exclude<GraphRelation, 'belongs-to'>;

export interface VisibleGraphEdge {
  from: GraphKey;
  relation: VisibleGraphRelation;
  to: GraphKey;
  members: GraphEdgeView[];
}

export interface VisibleGraph {
  nodes: GraphNodeView[];
  edges: VisibleGraphEdge[];
}

export function graphKeyId(key: GraphKey): string {
  return `${key.kind}:${key.id}`;
}

export function projectVisibleGraph(
  nodes: GraphNodeView[],
  edges: GraphEdgeView[],
  collapsedIds: ReadonlySet<string>,
): VisibleGraph {
  const nodeById = new Map<string, GraphNodeView>();
  for (const node of nodes) {
    const id = graphKeyId(node.key);
    if (nodeById.has(id)) throw new Error(`Duplicate graph node: ${id}`);
    nodeById.set(id, node);
  }

  for (const node of nodes) {
    const id = graphKeyId(node.key);
    const parentKind = node.parent?.kind;
    switch (node.key.kind) {
      case 'context':
      case 'code':
      case 'test':
        if (node.parent !== null) throw new Error(`Root graph node cannot have a parent: ${id}`);
        break;
      case 'capability':
        if (parentKind !== 'context') throw new Error(`Capability parent must be a context: ${id}`);
        break;
      case 'notion':
      case 'intent':
      case 'scenario':
        if (parentKind !== 'context' && parentKind !== 'capability') {
          throw new Error(`Domain graph node requires a container parent: ${id}`);
        }
        break;
      case 'constraint':
        if (
          node.parent !== null &&
          parentKind !== 'context' &&
          parentKind !== 'capability'
        ) {
          throw new Error(`Constraint parent must be a container or null: ${id}`);
        }
        break;
    }
  }

  const ancestorsById = new Map<string, GraphNodeView[]>();
  function ancestors(node: GraphNodeView): GraphNodeView[] {
    const id = graphKeyId(node.key);
    const cached = ancestorsById.get(id);
    if (cached) return cached;

    const result: GraphNodeView[] = [];
    const seen = new Set([id]);
    let parent = node.parent;
    while (parent) {
      const parentId = graphKeyId(parent);
      if (seen.has(parentId)) throw new Error(`Graph parent cycle at ${parentId}`);
      seen.add(parentId);

      const parentNode = nodeById.get(parentId);
      if (!parentNode) throw new Error(`Missing graph parent: ${parentId}`);
      if (parentNode.key.kind !== 'context' && parentNode.key.kind !== 'capability') {
        throw new Error(`Graph parent is not a container: ${parentId}`);
      }
      result.push(parentNode);
      parent = parentNode.parent;
    }
    ancestorsById.set(id, result);
    return result;
  }

  for (const node of nodes) ancestors(node);

  function isHidden(node: GraphNodeView): boolean {
    return ancestors(node).some((ancestor) => collapsedIds.has(graphKeyId(ancestor.key)));
  }

  function nearestVisibleKey(key: GraphKey): GraphKey {
    const node = nodeById.get(graphKeyId(key));
    if (!node) throw new Error(`Missing graph edge endpoint: ${graphKeyId(key)}`);
    if (!isHidden(node)) return node.key;

    const visibleAncestor = ancestors(node).find((ancestor) => !isHidden(ancestor));
    if (!visibleAncestor) throw new Error(`Graph node has no visible ancestor: ${graphKeyId(key)}`);
    return visibleAncestor.key;
  }

  const visibleNodes = nodes
    .filter((node) => !isHidden(node))
    .sort((left, right) => graphKeyId(left.key).localeCompare(graphKeyId(right.key)));
  const visibleEdges = new Map<string, VisibleGraphEdge>();
  for (const edge of edges) {
    if (edge.relation === 'belongs-to') continue;

    const from = nearestVisibleKey(edge.from);
    const to = nearestVisibleKey(edge.to);
    if (graphKeyId(from) === graphKeyId(to)) continue;

    const id = `${encodeURIComponent(graphKeyId(from))}:${edge.relation}:${encodeURIComponent(graphKeyId(to))}`;
    const existing = visibleEdges.get(id);
    if (existing) {
      existing.members.push(edge);
    } else {
      visibleEdges.set(id, { from, relation: edge.relation, to, members: [edge] });
    }
  }

  const sortedEdges = [...visibleEdges.values()].sort((left, right) => {
    const leftId = `${graphKeyId(left.from)}:${left.relation}:${graphKeyId(left.to)}`;
    const rightId = `${graphKeyId(right.from)}:${right.relation}:${graphKeyId(right.to)}`;
    return leftId.localeCompare(rightId);
  });

  return { nodes: visibleNodes, edges: sortedEdges };
}
