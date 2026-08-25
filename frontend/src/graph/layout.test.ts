import cytoscape from 'cytoscape';
import type { ElkNode } from 'elkjs/lib/elk.bundled.js';
import { describe, expect, it, vi } from 'vitest';

import {
  NODE_LAYOUT_MIN_HEIGHT,
  NODE_LAYOUT_MIN_WIDTH,
  runCompoundLayout,
} from './layout';

interface PendingLayout {
  graph: ElkNode;
  resolve: () => void;
}

function controlledEngine(): {
  engine: { layout: (graph: ElkNode) => Promise<ElkNode> };
  pending: PendingLayout[];
} {
  const pending: PendingLayout[] = [];
  return {
    engine: {
      layout: vi.fn(
        (graph: ElkNode) =>
          new Promise<ElkNode>((resolve) => {
            pending.push({ graph, resolve: () => resolve(graph) });
          }),
      ),
    },
    pending,
  };
}

describe('runCompoundLayout', () => {
  it('lets only the newest asynchronous layout own the final viewport', async () => {
    const cy = cytoscape({
      headless: true,
      elements: [
        { data: { id: 'a' } },
        { data: { id: 'b' } },
        { data: { id: 'a-b', source: 'a', target: 'b' } },
      ],
    });

    const fit = vi.spyOn(cy, 'fit');
    const { engine, pending } = controlledEngine();
    const stale = runCompoundLayout(cy, engine);
    cy.add({ data: { id: 'c' } });
    const current = runCompoundLayout(cy, engine);

    pending[1].resolve();
    await expect(current).resolves.toBe(true);
    expect(fit).toHaveBeenCalledTimes(1);

    pending[0].resolve();
    await expect(stale).resolves.toBe(false);
    expect(fit).toHaveBeenCalledTimes(1);
    cy.destroy();
  });

  it('propagates ELK failures instead of leaving layout callers pending', async () => {
    const cy = cytoscape({ headless: true, elements: [{ data: { id: 'a' } }] });
    const failure = new Error('ELK failed');

    await expect(
      runCompoundLayout(cy, { layout: vi.fn().mockRejectedValue(failure) }),
    ).rejects.toBe(failure);
    cy.destroy();
  });

  it('does not touch Cytoscape after destruction while ELK is in flight', async () => {
    const cy = cytoscape({ headless: true, elements: [{ data: { id: 'a' } }] });
    const fit = vi.spyOn(cy, 'fit');
    const { engine, pending } = controlledEngine();
    const running = runCompoundLayout(cy, engine);

    cy.destroy();
    pending[0].resolve();

    await expect(running).resolves.toBe(false);
    expect(fit).not.toHaveBeenCalled();
  });

  it('keeps an isolated context constraint compact and long sibling labels separated', async () => {
    const cy = cytoscape({
      headless: true,
      styleEnabled: true,
      style: [
        {
          selector: 'node',
          style: {
            width: 44,
            height: 44,
            label: 'data(label)',
            'font-size': 10,
            'text-wrap': 'ellipsis',
            'text-max-width': '120px',
            'text-valign': 'bottom',
            'text-margin-y': 8,
          },
        },
        { selector: 'node[container]', style: { padding: '24px' } },
      ],
      elements: [
        { data: { id: 'context:billing', label: 'Billing', container: true } },
        {
          data: {
            id: 'capability:billing/payments',
            label: 'Payments',
            container: true,
            parent: 'context:billing',
          },
        },
        {
          data: {
            id: 'capability:billing/invoicing',
            label: 'Invoicing',
            container: true,
            parent: 'context:billing',
          },
        },
        {
          data: {
            id: 'notion:Customer',
            label: 'Customer receiving the issued invoice',
            parent: 'context:billing',
          },
        },
        {
          data: {
            id: 'notion:Invoice',
            label: 'Invoice issued to a customer for delivered work',
            parent: 'capability:billing/payments',
          },
        },
        {
          data: {
            id: 'intent:INT-0042',
            label: 'A full payment settles the outstanding invoice balance',
            parent: 'capability:billing/payments',
          },
        },
        {
          data: {
            id: 'scenario:SCN-0107',
            label: 'The customer pays the complete remaining balance',
            parent: 'capability:billing/payments',
          },
        },
        {
          data: {
            id: 'constraint:CON-0011',
            label: 'Payments must preserve the accounting invariant',
            parent: 'context:billing',
          },
        },
        {
          data: {
            id: 'code:src/invoice.rs',
            label: 'src/invoice.rs',
          },
        },
        ...Array.from({ length: 12 }, (_, index) => ({
          data: {
            id: `scenario:SCN-${String(index + 200).padStart(4, '0')}`,
            label: `A deliberately long settlement example number ${index + 1}`,
            parent: 'capability:billing/payments',
          },
        })),
        ...Array.from({ length: 10 }, (_, index) => ({
          data: {
            id: `intent:INT-${String(index + 300).padStart(4, '0')}`,
            label: `Issue invoice variant number ${index + 1} to the customer`,
            parent: 'capability:billing/invoicing',
          },
        })),
        {
          data: {
            id: 'uses',
            source: 'intent:INT-0042',
            target: 'notion:Invoice',
          },
        },
        {
          data: {
            id: 'verifies',
            source: 'scenario:SCN-0107',
            target: 'intent:INT-0042',
          },
        },
        {
          data: {
            id: 'implements',
            source: 'code:src/invoice.rs',
            target: 'intent:INT-0042',
          },
        },
        ...Array.from({ length: 12 }, (_, index) => ({
          data: {
            id: `uses-${index}`,
            source: `scenario:SCN-${String(index + 200).padStart(4, '0')}`,
            target: 'notion:Invoice',
          },
        })),
        ...Array.from({ length: 10 }, (_, index) => ({
          data: {
            id: `customer-${index}`,
            source: `intent:INT-${String(index + 300).padStart(4, '0')}`,
            target: 'notion:Customer',
          },
        })),
      ],
    });

    await runCompoundLayout(cy);

    const contextBox = cy.getElementById('context:billing').boundingBox({
      includeEdges: false,
      includeLabels: true,
    });
    expect(contextBox.w).toBeLessThan(900);
    // Twenty-two 80px-high label reservations need vertical room in a left-to-right diagram,
    // but a disconnected child must not inflate the container beyond their packed footprint.
    expect(contextBox.h).toBeLessThan(3_000);

    const siblings = cy.getElementById('capability:billing/payments').children().toArray();
    for (let left = 0; left < siblings.length; left += 1) {
      for (let right = left + 1; right < siblings.length; right += 1) {
        const aPosition = siblings[left].position();
        const bPosition = siblings[right].position();
        const a = {
          x1: aPosition.x - NODE_LAYOUT_MIN_WIDTH / 2,
          x2: aPosition.x + NODE_LAYOUT_MIN_WIDTH / 2,
          y1: aPosition.y - NODE_LAYOUT_MIN_HEIGHT / 2,
          y2: aPosition.y + NODE_LAYOUT_MIN_HEIGHT / 2,
        };
        const b = {
          x1: bPosition.x - NODE_LAYOUT_MIN_WIDTH / 2,
          x2: bPosition.x + NODE_LAYOUT_MIN_WIDTH / 2,
          y1: bPosition.y - NODE_LAYOUT_MIN_HEIGHT / 2,
          y2: bPosition.y + NODE_LAYOUT_MIN_HEIGHT / 2,
        };
        const overlap = a.x1 < b.x2 && a.x2 > b.x1 && a.y1 < b.y2 && a.y2 > b.y1;
        expect(overlap, `${siblings[left].id()} overlaps ${siblings[right].id()}`).toBe(false);
      }
    }
    cy.destroy();
  });
});
