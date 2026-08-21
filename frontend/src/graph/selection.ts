import { ref, watch, type Ref } from 'vue';

import type { GraphEdgeView, GraphNodeView } from '../data/types';
import type { GraphSelection } from './elements';

export interface GraphSelectionState {
  selected: Ref<GraphSelection | null>;
  setSelection: (selection: GraphSelection | null) => void;
}

export function useGraphSelection(
  nodes: Readonly<Ref<GraphNodeView[]>>,
  edges: Readonly<Ref<GraphEdgeView[]>>,
): GraphSelectionState {
  const selected = ref<GraphSelection | null>(null);

  function setSelection(selection: GraphSelection | null): void {
    selected.value = selection;
  }

  watch([nodes, edges], () => setSelection(null));

  return { selected, setSelection };
}
