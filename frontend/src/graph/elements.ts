import type { ElementDefinition } from 'cytoscape';

import type {
  GraphEdgeView,
  GraphKey,
  GraphNodeView,
  GraphRelation,
} from '../data/types';

export type RelationFilter = 'all' | GraphRelation;

export interface GraphNodeSelection {
  type: 'node';
  entity: GraphKey;
  label: string;
}

export interface GraphEdgeSelection {
  type: 'edge';
  relation: GraphRelation;
  source: GraphKey;
  target: GraphKey;
}

export type GraphSelection = GraphNodeSelection | GraphEdgeSelection;

export function nodeId(key: GraphKey): string {
  return `${key.kind}:${key.id}`;
}

function edgeId(edge: GraphEdgeView): string {
  const source = encodeURIComponent(nodeId(edge.from));
  const target = encodeURIComponent(nodeId(edge.to));
  return `edge:${source}:${edge.relation}:${target}`;
}

export function buildGraphElements(
  nodes: GraphNodeView[],
  edges: GraphEdgeView[],
): ElementDefinition[] {
  const edgeIds = edges.map(edgeId);
  const edgeIdCounts = new Map<string, number>();
  for (const id of edgeIds) edgeIdCounts.set(id, (edgeIdCounts.get(id) ?? 0) + 1);
  const edgeIdOccurrences = new Map<string, number>();

  return [
    ...nodes.map<ElementDefinition>((node) => ({
      group: 'nodes',
      data: {
        id: nodeId(node.key),
        kind: node.key.kind,
        entityId: node.key.id,
        label: node.label,
      },
    })),
    ...edges.map<ElementDefinition>((edge, index) => {
      const baseId = edgeIds[index];
      const occurrence = edgeIdOccurrences.get(baseId) ?? 0;
      edgeIdOccurrences.set(baseId, occurrence + 1);
      const id = edgeIdCounts.get(baseId) === 1 ? baseId : `${baseId}:${occurrence}`;

      return {
        group: 'edges',
        data: {
          id,
          source: nodeId(edge.from),
          target: nodeId(edge.to),
          relation: edge.relation,
          sourceKind: edge.from.kind,
          sourceId: edge.from.id,
          targetKind: edge.to.kind,
          targetId: edge.to.id,
        },
      };
    }),
  ];
}

export function dimmedElementIds(
  elements: ElementDefinition[],
  filter: RelationFilter,
): Set<string> {
  if (filter === 'all') return new Set();

  const connectedNodeIds = new Set<string>();
  for (const element of elements) {
    if (element.group !== 'edges' || element.data.relation !== filter) continue;
    connectedNodeIds.add(element.data.source as string);
    connectedNodeIds.add(element.data.target as string);
  }

  const dimmed = new Set<string>();
  for (const element of elements) {
    const id = element.data.id as string;
    if (element.group === 'nodes') {
      if (!connectedNodeIds.has(id)) dimmed.add(id);
    } else if (element.data.relation !== filter) {
      dimmed.add(id);
    }
  }
  return dimmed;
}
