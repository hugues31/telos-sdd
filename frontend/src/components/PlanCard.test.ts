import { describe, expect, it } from 'vitest';
import { createSSRApp, defineComponent, h } from 'vue';
import { renderToString } from '@vue/server-renderer';
import PlanCard from './PlanCard.vue';
import type { PlanView } from '../data/types';

function plan(done: number, total: number, state: PlanView['state'] = 'running'): PlanView {
  return {
    id: 'PLN-00000000-0000-0000-0000-000000000001', title: 'Document setup',
    goal: 'Contributors can install the project', revision: 2, version: 7,
    digest: 'sha256:fixture', state, approved: true,
    progress: { done, total, percent: total ? Math.floor(100 * done / total) : null },
    current_task: 'TSK-004', last_activity: '2026-09-20T12:00:00Z', tasks: [],
    brief: { summary: 'Installation guide', decisions: [], questions: [], exclusions: [] },
    success_criteria: [], scope: ['README.md'], events: [],
  };
}

async function render(value: PlanView) {
  const app = createSSRApp({ render: () => h(PlanCard, { plan: value }) });
  app.component('RouterLink', defineComponent({
    props: { to: String },
    setup: (props, { slots }) => () => h('a', { href: props.to }, slots.default?.()),
  }));
  return renderToString(app);
}

describe('native plan cards', () => {
  it('shows an accessible progress meter and the plan destination', async () => {
    const html = await render(plan(3, 5));
    expect(html).toContain('60% · 3 of 5 tasks done');
    expect(html).toContain('aria-valuenow="60"');
    expect(html).toContain('/plan/PLN-00000000-0000-0000-0000-000000000001');
    expect(html).toContain('TSK-004');
  });
  it('does not invent a percentage for an empty plan', async () => {
    const html = await render(plan(0, 0, 'draft'));
    expect(html).toContain('To be planned');
    expect(html).not.toContain('role="progressbar"');
  });
  it('keeps final validation visible when every task is finished', async () => {
    expect(await render(plan(5, 5))).toContain('final validation pending');
    expect(await render(plan(5, 5, 'completed'))).not.toContain('final validation pending');
  });
});
