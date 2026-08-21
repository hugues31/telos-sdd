import type { Position } from 'cytoscape';

const FALLBACK_SPACING = 96;
const FALLBACK_COLUMNS = 4;

export function replacementNodePositions(
  nodeIds: string[],
  previous: ReadonlyMap<string, Position>,
): Map<string, Position> {
  const positions = new Map<string, Position>();
  const unseen: string[] = [];

  for (const id of nodeIds) {
    const position = previous.get(id);
    if (position) {
      positions.set(id, { ...position });
    } else {
      unseen.push(id);
    }
  }

  const retained = [...positions.values()];
  const startX = retained.length
    ? Math.max(...retained.map((position) => position.x)) + FALLBACK_SPACING
    : 0;
  const startY = retained.length ? Math.min(...retained.map((position) => position.y)) : 0;

  unseen.forEach((id, index) => {
    positions.set(id, {
      x: startX + (index % FALLBACK_COLUMNS) * FALLBACK_SPACING,
      y: startY + Math.floor(index / FALLBACK_COLUMNS) * FALLBACK_SPACING,
    });
  });

  return positions;
}
