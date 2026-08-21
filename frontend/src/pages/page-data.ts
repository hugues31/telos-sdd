import type { NotionKind, NotionView } from '../data/types';

export const notionKinds: NotionKind[] = ['actor', 'entity', 'value', 'event', 'state'];

export interface NotionGroup {
  kind: NotionKind;
  notions: NotionView[];
}

export function filterNotions(notions: NotionView[], query: string): NotionView[] {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return notions;

  return notions.filter((notion) =>
    [notion.name, notion.definition, notion.kind, notion.canonical].some((value) =>
      value.toLocaleLowerCase().includes(needle),
    ),
  );
}

export function groupNotions(notions: NotionView[]): NotionGroup[] {
  return notionKinds
    .map((kind) => ({ kind, notions: notions.filter((notion) => notion.kind === kind) }))
    .filter((group) => group.notions.length > 0);
}

export function percentage(value: number, total: number): number {
  return total === 0 ? 0 : Math.round((value / total) * 100);
}
