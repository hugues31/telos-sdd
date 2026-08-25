<script setup lang="ts">
import cytoscape, {
  type Core,
  type SingularElementReturnValue,
} from 'cytoscape';
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';

import type { GraphEdgeView, GraphKeyKind, GraphNodeView } from '../data/types';
import {
  buildGraphElements,
  dimmedElementIds,
  graphSelectionId,
  nodeId,
  type GraphSelection,
  type RelationFilter,
} from './elements';
import { runCompoundLayout } from './layout';
import type { VisibleGraphEdge, VisibleGraphRelation } from './projection';
import { collapsedIdsSignature } from './collapse';
import { buildGraphStylesheet } from './stylesheet';

const props = defineProps<{
  nodes: GraphNodeView[];
  edges: VisibleGraphEdge[];
  relationFilter: RelationFilter;
  collapsedIds: string[];
  selection: GraphSelection | null;
}>();

const emit = defineEmits<{
  select: [selection: GraphSelection | null];
  'toggle-container': [id: string];
  'expand-all': [];
  'collapse-all': [];
}>();

const container = ref<HTMLElement | null>(null);
let cy: Core | null = null;
let resizeObserver: ResizeObserver | null = null;
let themeObserver: MutationObserver | null = null;

function selectionFromElement(element: SingularElementReturnValue): GraphSelection {
  if (element.group() === 'nodes') {
    return {
      type: 'node',
      entity: {
        kind: element.data('kind') as GraphKeyKind,
        id: element.data('entityId') as string,
      },
      label: element.data('rawLabel') as string,
    };
  }

  return {
    type: 'edge',
    relation: element.data('relation') as VisibleGraphRelation,
    source: {
      kind: element.data('sourceKind') as GraphKeyKind,
      id: element.data('sourceId') as string,
    },
    target: {
      kind: element.data('targetKind') as GraphKeyKind,
      id: element.data('targetId') as string,
    },
    members: element.data('members') as GraphEdgeView[],
  };
}

function collapsedIdSet(): Set<string> {
  return new Set(props.collapsedIds);
}

function applyFilter(filter: RelationFilter): void {
  if (!cy) return;
  const dimmedIds = dimmedElementIds(
    buildGraphElements(props.nodes, props.edges, collapsedIdSet()),
    filter,
  );
  cy.batch(() => {
    cy?.elements().removeClass('dimmed');
    for (const id of dimmedIds) cy?.getElementById(id).addClass('dimmed');
  });
}

function applySelectedHighlight(selection: GraphSelection | null): void {
  if (!cy) return;
  cy.elements().unselect();
  if (selection) cy.getElementById(graphSelectionId(selection)).select();
}

function fitGraph(): void {
  cy?.fit(undefined, 32);
}

function relayoutGraph(): void {
  if (!cy) return;
  void runCompoundLayout(cy).catch((error: unknown) => {
    console.error('ELK graph layout failed', error);
  });
}

watch(
  () => props.relationFilter,
  (filter) => applyFilter(filter),
);

watch(
  () => props.selection,
  (selection) => applySelectedHighlight(selection),
);

watch(
  [
    () => props.nodes,
    () => props.edges,
    () => collapsedIdsSignature(props.collapsedIds),
  ],
  () => {
    if (!cy) return;
    cy.batch(() => {
      cy?.elements().remove();
      cy?.add(buildGraphElements(props.nodes, props.edges, collapsedIdSet()));
    });
    relayoutGraph();
    applyFilter(props.relationFilter);
    applySelectedHighlight(props.selection);
  },
);

onMounted(() => {
  if (!container.value) return;

  cy = cytoscape({
    container: container.value,
    elements: buildGraphElements(props.nodes, props.edges, collapsedIdSet()),
    style: buildGraphStylesheet(),
    layout: { name: 'preset' },
    selectionType: 'single',
    minZoom: 0.005,
    maxZoom: 4,
  });

  cy.on('tap', 'node, edge', (event) => {
    const element = event.target as SingularElementReturnValue;
    emit('select', selectionFromElement(element));

    if (!element.isNode()) return;
    const kind = element.data('kind') as GraphKeyKind;
    if (kind !== 'context' && kind !== 'capability') return;

    const bounds = element.renderedBoundingBox();
    const collapsed = element.data('collapsed') as boolean;
    if (collapsed || event.renderedPosition.y <= bounds.y1 + 32) {
      emit('toggle-container', nodeId({ kind, id: element.data('entityId') as string }));
    }
  });
  cy.on('tap', (event) => {
    if (event.target !== cy) return;
    cy?.elements().unselect();
    emit('select', null);
  });
  cy.on('mouseover', 'node, edge', (event) => {
    (event.target as SingularElementReturnValue).addClass('hovered');
  });
  cy.on('mouseout', 'node, edge', (event) => {
    (event.target as SingularElementReturnValue).removeClass('hovered');
  });

  applyFilter(props.relationFilter);
  relayoutGraph();
  applySelectedHighlight(props.selection);

  resizeObserver = new ResizeObserver(() => cy?.resize());
  resizeObserver.observe(container.value);

  themeObserver = new MutationObserver(() => {
    cy?.style(buildGraphStylesheet()).update();
  });
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme'],
  });
});

onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  themeObserver?.disconnect();
  cy?.destroy();
  resizeObserver = null;
  themeObserver = null;
  cy = null;
});
</script>

<template>
  <div class="cyto-graph">
    <div class="cyto-graph__toolbar" aria-label="Graph controls">
      <button type="button" data-graph-action="fit" @click="fitGraph">Fit</button>
      <button type="button" data-graph-action="relayout" @click="relayoutGraph">Re-layout</button>
      <button type="button" data-graph-action="expand-all" @click="emit('expand-all')">
        Expand all
      </button>
      <button type="button" data-graph-action="collapse-all" @click="emit('collapse-all')">
        Collapse all
      </button>
    </div>
    <div
      ref="container"
      class="cyto-graph__canvas"
      role="application"
      aria-label="Interactive dependency graph"
    ></div>
  </div>
</template>

<style scoped>
.cyto-graph {
  position: relative;
  overflow: hidden;
  min-width: 0;
  border: 1px solid var(--color-border);
  border-radius: 0.75rem;
  background: var(--color-surface);
}

.cyto-graph__toolbar {
  position: absolute;
  z-index: 2;
  top: 0.75rem;
  right: 0.75rem;
  display: flex;
  gap: 0.5rem;
}

.cyto-graph__toolbar button {
  border: 1px solid var(--color-border);
  border-radius: 0.375rem;
  background: var(--color-surface-raised);
  padding: 0.375rem 0.625rem;
  cursor: pointer;
  font-size: 0.8125rem;
}

.cyto-graph__toolbar button:hover {
  border-color: var(--color-primary);
}

.cyto-graph__canvas {
  width: 100%;
  height: min(68vh, 44rem);
  min-height: 30rem;
}

@media (max-width: 48rem) {
  .cyto-graph__canvas {
    height: 30rem;
    min-height: 24rem;
  }
}
</style>
