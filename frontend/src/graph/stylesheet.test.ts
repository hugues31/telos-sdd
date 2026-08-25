import { afterEach, describe, expect, it, vi } from 'vitest';

import { buildGraphStylesheet } from './stylesheet';

describe('buildGraphStylesheet', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('keeps rendered node labels to one bounded ellipsized line', () => {
    vi.stubGlobal(
      'getComputedStyle',
      () =>
        ({
          getPropertyValue: () => '#000',
        }) as unknown as CSSStyleDeclaration,
    );

    const stylesheet = buildGraphStylesheet({} as Element);
    const nodeStyle = (
      stylesheet as Array<{ selector: string; style?: Record<string, unknown> }>
    ).find((entry) => entry.selector === 'node')?.style;

    expect(nodeStyle).toMatchObject({
      'text-wrap': 'ellipsis',
      'text-max-width': '120px',
      'text-overflow-wrap': 'anywhere',
    });
  });
});
