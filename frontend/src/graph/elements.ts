import type { ElementDefinition } from 'cytoscape';

import type { GraphEdgeView, GraphKey, GraphNodeView } from '../data/types';
import type { VisibleGraphEdge, VisibleGraphRelation } from './projection';

export type RelationFilter = 'all' | VisibleGraphRelation;

export interface GraphNodeSelection {
  type: 'node';
  entity: GraphKey;
  label: string;
}

export interface GraphEdgeSelection {
  type: 'edge';
  relation: VisibleGraphRelation;
  source: GraphKey;
  target: GraphKey;
  members: GraphEdgeView[];
}

export type GraphSelection = GraphNodeSelection | GraphEdgeSelection;

export function nodeId(key: GraphKey): string {
  return `${key.kind}:${key.id}`;
}

function edgeId(edge: VisibleGraphEdge): string {
  return graphEdgeId(edge.from, edge.relation, edge.to);
}

function graphEdgeId(from: GraphKey, relation: VisibleGraphRelation, to: GraphKey): string {
  const source = encodeURIComponent(nodeId(from));
  const target = encodeURIComponent(nodeId(to));
  return `edge:${source}:${relation}:${target}`;
}

export function graphSelectionId(selection: GraphSelection): string {
  return selection.type === 'node'
    ? nodeId(selection.entity)
    : graphEdgeId(selection.source, selection.relation, selection.target);
}

const MAX_RENDERED_LABEL_LENGTH = 40;

function renderedNodeLabel(label: string): string {
  if (label.length <= MAX_RENDERED_LABEL_LENGTH) return label;
  return `${label.slice(0, MAX_RENDERED_LABEL_LENGTH - 1).trimEnd()}…`;
}

export function buildGraphElements(
  nodes: GraphNodeView[],
  edges: VisibleGraphEdge[],
  collapsedIds: ReadonlySet<string> = new Set(),
): ElementDefinition[] {
  const edgeIds = edges.map(edgeId);
  const edgeIdCounts = new Map<string, number>();
  for (const id of edgeIds) edgeIdCounts.set(id, (edgeIdCounts.get(id) ?? 0) + 1);
  const edgeIdOccurrences = new Map<string, number>();

  const orderedNodes = nodes
    .map((node, index) => ({ node, index }))
    .sort((left, right) => {
      const rank = (node: GraphNodeView): number =>
        node.key.kind === 'context' ? 0 : node.key.kind === 'capability' ? 1 : 2;
      return rank(left.node) - rank(right.node) || left.index - right.index;
    })
    .map(({ node }) => node);

  return [
    ...orderedNodes.map<ElementDefinition>((node) => {
      const id = nodeId(node.key);
      const isContainer = node.key.kind === 'context' || node.key.kind === 'capability';
      const displayLabel = renderedNodeLabel(node.label);
      return {
        group: 'nodes',
        data: {
          id,
          kind: node.key.kind,
          entityId: node.key.id,
          rawLabel: node.label,
          label: isContainer
            ? `${collapsedIds.has(id) ? '+' : '−'} ${displayLabel}`
            : displayLabel,
          ...(isContainer ? { container: true } : {}),
          ...(isContainer && collapsedIds.has(id) ? { collapsed: true } : {}),
          ...(node.parent ? { parent: nodeId(node.parent) } : {}),
        },
      };
    }),
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
          count: edge.members.length,
          displayLabel:
            edge.members.length > 1 ? `${edge.relation} ×${edge.members.length}` : edge.relation,
          members: edge.members,
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
      if (element.data.kind === 'context' || element.data.kind === 'capability') continue;
      if (!connectedNodeIds.has(id)) dimmed.add(id);
    } else if (element.data.relation !== filter) {
      dimmed.add(id);
    }
  }
  return dimmed;
}
