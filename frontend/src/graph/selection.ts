import { ref, watch, type Ref } from 'vue';

import type { GraphNodeView } from '../data/types';
import type { GraphSelection } from './elements';
import { nodeId } from './elements';
import type { VisibleGraphEdge } from './projection';

export interface GraphSelectionState {
  selected: Ref<GraphSelection | null>;
  setSelection: (selection: GraphSelection | null) => void;
}

export function useGraphSelection(
  nodes: Readonly<Ref<GraphNodeView[]>>,
  edges: Readonly<Ref<VisibleGraphEdge[]>>,
): GraphSelectionState {
  const selected = ref<GraphSelection | null>(null);

  function setSelection(selection: GraphSelection | null): void {
    selected.value = selection;
  }

  watch([nodes, edges], () => {
    const current = selected.value;
    if (!current) return;

    if (current.type === 'node') {
      if (!nodes.value.some((node) => nodeId(node.key) === nodeId(current.entity))) {
        setSelection(null);
      }
      return;
    }

    const replacement = edges.value.find(
      (edge) =>
        edge.relation === current.relation &&
        nodeId(edge.from) === nodeId(current.source) &&
        nodeId(edge.to) === nodeId(current.target),
    );
    if (!replacement) {
      setSelection(null);
      return;
    }
    selected.value = { ...current, members: replacement.members };
  });

  return { selected, setSelection };
}
