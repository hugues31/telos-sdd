import { effectScope, nextTick, ref } from 'vue';
import { describe, expect, it } from 'vitest';

import type { GraphEdgeView, GraphNodeView } from '../data/types';
import type { VisibleGraphEdge } from './projection';
import { useGraphSelection } from './selection';

const semanticEdge: GraphEdgeView = {
  from: { kind: 'intent', id: 'INT-0042' },
  relation: 'uses',
  to: { kind: 'notion', id: 'Invoice' },
};

function visibleEdge(...members: GraphEdgeView[]): VisibleGraphEdge {
  return { ...semanticEdge, relation: 'uses', members };
}

describe('useGraphSelection', () => {
  it('clears a parent-owned selection when replacement transitions through no nodes', async () => {
    const nodes = ref<GraphNodeView[]>([
      {
        key: { kind: 'intent', id: 'INT-0042' },
        label: 'Settle invoices',
        parent: { kind: 'capability', id: 'billing/settlement' },
      },
    ]);
    const edges = ref<VisibleGraphEdge[]>([]);
    const scope = effectScope();
    const state = scope.run(() => useGraphSelection(nodes, edges));
    if (!state) throw new Error('selection scope did not start');

    state.setSelection({
      type: 'node',
      entity: { kind: 'intent', id: 'INT-0042' },
      label: 'Settle invoices',
    });
    nodes.value = [];
    await nextTick();

    expect(state.selected.value).toBeNull();
    scope.stop();
  });

  it('preserves a node selection across equivalent visible projections', async () => {
    const node = {
      key: { kind: 'intent', id: 'INT-0042' },
      label: 'Settle invoices',
      parent: { kind: 'capability', id: 'billing/settlement' },
    } satisfies GraphNodeView;
    const nodes = ref<GraphNodeView[]>([node]);
    const edges = ref<VisibleGraphEdge[]>([]);
    const scope = effectScope();
    const state = scope.run(() => useGraphSelection(nodes, edges));
    if (!state) throw new Error('selection scope did not start');

    state.setSelection({ type: 'node', entity: node.key, label: node.label });
    nodes.value = [{ ...node }];
    await nextTick();

    expect(state.selected.value?.type).toBe('node');
    scope.stop();
  });

  it('preserves an aggregated edge selection while its visible edge still exists', async () => {
    const nodes = ref<GraphNodeView[]>([]);
    const edges = ref<VisibleGraphEdge[]>([visibleEdge(semanticEdge, semanticEdge)]);
    const scope = effectScope();
    const state = scope.run(() => useGraphSelection(nodes, edges));
    if (!state) throw new Error('selection scope did not start');

    state.setSelection({
      type: 'edge',
      relation: 'uses',
      source: semanticEdge.from,
      target: semanticEdge.to,
      members: [semanticEdge, semanticEdge],
    });
    edges.value = [visibleEdge(semanticEdge)];
    await nextTick();

    expect(state.selected.value).toMatchObject({ type: 'edge', relation: 'uses' });
    expect(state.selected.value?.type === 'edge' && state.selected.value.members).toHaveLength(1);
    scope.stop();
  });

  it('clears an edge selection when collapse removes its visible relation', async () => {
    const nodes = ref<GraphNodeView[]>([]);
    const edges = ref<VisibleGraphEdge[]>([visibleEdge(semanticEdge)]);
    const scope = effectScope();
    const state = scope.run(() => useGraphSelection(nodes, edges));
    if (!state) throw new Error('selection scope did not start');

    state.setSelection({
      type: 'edge',
      relation: 'uses',
      source: semanticEdge.from,
      target: semanticEdge.to,
      members: [semanticEdge],
    });
    edges.value = [];
    await nextTick();

    expect(state.selected.value).toBeNull();
    scope.stop();
  });
});
