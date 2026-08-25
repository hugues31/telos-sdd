import type { Core, NodeSingular } from 'cytoscape';
import ELK, { type ElkNode } from 'elkjs/lib/elk.bundled.js';

const layoutGeneration = new WeakMap<Core, number>();

export const NODE_LAYOUT_MIN_WIDTH = 120;
export const NODE_LAYOUT_MIN_HEIGHT = 80;

export const ELK_LAYOUT_OPTIONS = {
  padding: 48,
  nodeDimensionsIncludeLabels: true,
  elk: {
    algorithm: 'layered',
    'elk.direction': 'RIGHT',
    'elk.hierarchyHandling': 'INCLUDE_CHILDREN',
    'elk.spacing.nodeNode': '24',
    'elk.spacing.componentComponent': '64',
    'elk.layered.spacing.nodeNodeBetweenLayers': '64',
    'elk.layered.crossingMinimization.greedySwitch.type': 'TWO_SIDED',
  },
} as const;

export interface ElkLayoutEngine {
  layout(graph: ElkNode): Promise<ElkNode>;
}

function elkNode(node: NodeSingular): ElkNode {
  const result: ElkNode = { id: node.id() };
  if (node.isParent()) return result;

  const dimensions = node.layoutDimensions({
    nodeDimensionsIncludeLabels: ELK_LAYOUT_OPTIONS.nodeDimensionsIncludeLabels,
  });
  const isContainer = Boolean(node.data('container'));
  result.width = Math.max(dimensions.w, NODE_LAYOUT_MIN_WIDTH);
  result.height = Math.max(dimensions.h, isContainer ? 44 : NODE_LAYOUT_MIN_HEIGHT);
  return result;
}

function elkGraph(cy: Core): ElkNode {
  const graph: ElkNode = {
    id: 'root',
    children: [],
    edges: [],
    layoutOptions: { ...ELK_LAYOUT_OPTIONS.elk },
  };
  const byId = new Map<string, ElkNode>();

  cy.nodes().forEach((node) => {
    byId.set(node.id(), elkNode(node));
  });
  cy.nodes().forEach((node) => {
    const child = byId.get(node.id());
    if (!child) return;

    const parent = node.parent();
    if (parent.empty()) {
      graph.children?.push(child);
      return;
    }

    const parentNode = byId.get(parent[0].id());
    if (!parentNode) return;
    parentNode.children ??= [];
    parentNode.children.push(child);
  });

  cy.edges().forEach((edge) => {
    graph.edges?.push({
      id: edge.id(),
      sources: [edge.source().id()],
      targets: [edge.target().id()],
    });
  });

  return graph;
}

function leafPositions(
  node: ElkNode,
  positions: Map<string, { x: number; y: number }>,
  parentX = 0,
  parentY = 0,
): void {
  const x = parentX + (node.x ?? 0);
  const y = parentY + (node.y ?? 0);
  if (node.children?.length) {
    for (const child of node.children) leafPositions(child, positions, x, y);
    return;
  }

  if (node.id === 'root') return;
  positions.set(node.id, {
    x: x + (node.width ?? 0) / 2,
    y: y + (node.height ?? 0) / 2,
  });
}

export async function runCompoundLayout(
  cy: Core,
  engine: ElkLayoutEngine = new ELK(),
): Promise<boolean> {
  const generation = (layoutGeneration.get(cy) ?? 0) + 1;
  layoutGeneration.set(cy, generation);
  const graph = elkGraph(cy);

  const result = await engine.layout(graph);

  if (cy.destroyed() || layoutGeneration.get(cy) !== generation) return false;

  const positions = new Map<string, { x: number; y: number }>();
  leafPositions(result, positions);
  cy.batch(() => {
    for (const [id, position] of positions) cy.getElementById(id).position(position);
  });
  cy.fit(undefined, ELK_LAYOUT_OPTIONS.padding);
  return true;
}
