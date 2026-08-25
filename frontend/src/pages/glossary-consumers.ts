import type { GraphKey } from '../data/types';

export type ConsumerKind = 'intent' | 'scenario';
export type ConsumerSort = 'kind' | 'id' | 'title';
export type SortDirection = 'asc' | 'desc';

export interface ConsumerRow {
  kind: ConsumerKind;
  id: string;
  title: string;
  entity: GraphKey;
}

interface TitledEntity {
  title: string;
}

export function consumerRows(
  consumers: readonly GraphKey[],
  intents: ReadonlyMap<string, TitledEntity>,
  scenarios: ReadonlyMap<string, TitledEntity>,
): ConsumerRow[] {
  return consumers.flatMap((entity): ConsumerRow[] => {
    if (entity.kind === 'intent') {
      return [
        {
          kind: entity.kind,
          id: entity.id,
          title: intents.get(entity.id)?.title ?? entity.id,
          entity,
        },
      ];
    }
    if (entity.kind === 'scenario') {
      return [
        {
          kind: entity.kind,
          id: entity.id,
          title: scenarios.get(entity.id)?.title ?? entity.id,
          entity,
        },
      ];
    }
    return [];
  });
}

export function filterConsumerRows(
  rows: readonly ConsumerRow[],
  kind: '' | ConsumerKind,
): ConsumerRow[] {
  return kind === '' ? [...rows] : rows.filter((row) => row.kind === kind);
}

export function sortConsumerRows(
  rows: readonly ConsumerRow[],
  sort: ConsumerSort,
  direction: SortDirection,
): ConsumerRow[] {
  const multiplier = direction === 'asc' ? 1 : -1;
  return [...rows].sort((left, right) => {
    const primary = left[sort].localeCompare(right[sort]);
    return multiplier * (primary || left.id.localeCompare(right.id));
  });
}
