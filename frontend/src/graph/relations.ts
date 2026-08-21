import { GRAPH_RELATIONS, type GraphEdgeView, type GraphRelation } from '../data/types';
import type { RelationFilter } from './elements';

export interface RelationOption {
  value: RelationFilter;
  label: string;
}

const RELATION_OPTIONS: readonly RelationOption[] = [
  { value: 'all', label: 'All' },
  ...GRAPH_RELATIONS.map((value) => ({ value, label: relationLabel(value) })),
];

export function relationLabel(relation: GraphRelation): string {
  return relation.charAt(0).toUpperCase() + relation.slice(1);
}

/** Options are a model-independent part of the graph contract, even when sparse. */
export function relationOptionsFor(_edges: readonly GraphEdgeView[]): readonly RelationOption[] {
  return RELATION_OPTIONS;
}
