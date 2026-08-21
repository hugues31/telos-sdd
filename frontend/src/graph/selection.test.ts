import { effectScope, nextTick, ref } from 'vue';
import { describe, expect, it } from 'vitest';

import type { GraphEdgeView, GraphNodeView } from '../data/types';
import { useGraphSelection } from './selection';

describe('useGraphSelection', () => {
  it('clears a parent-owned selection when replacement transitions through no nodes', async () => {
    const nodes = ref<GraphNodeView[]>([
      { key: { kind: 'intent', id: 'INT-0042' }, label: 'Settle invoices' },
    ]);
    const edges = ref<GraphEdgeView[]>([]);
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
});
