import type { RouteLocationRaw } from 'vue-router';

import type { GraphKey } from '../data/types';

export function entityDestination(
  entity: GraphKey,
  scenarioParent?: string,
): RouteLocationRaw | null {
  switch (entity.kind) {
    case 'context':
      return { name: 'contexts', hash: `#context-${entity.id}` };
    case 'capability':
      return { name: 'contexts', hash: `#capability-${entity.id.replace('/', '-')}` };
    case 'intent':
      return { name: 'intent-detail', params: { id: entity.id } };
    case 'scenario':
      return scenarioParent
        ? {
            name: 'intent-detail',
            params: { id: scenarioParent },
            hash: `#scenario-${entity.id}`,
          }
        : null;
    case 'notion':
      return { name: 'glossary', hash: `#notion-${entity.id}` };
    case 'constraint':
      return { name: 'coverage', hash: `#constraint-${entity.id}` };
    case 'code':
    case 'test':
      return {
        name: 'graph',
        query: { focusKind: entity.kind, focusId: entity.id },
      };
  }
}
