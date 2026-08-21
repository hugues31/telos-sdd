import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const args = process.argv.slice(2);
let outputPath = null;
let intentCount = 300;
let forceBillingFixture = false;

for (let index = 0; index < args.length; index += 1) {
  const argument = args[index];
  if (argument === '--intents') {
    intentCount = Number(args[index + 1]);
    index += 1;
  } else if (argument === '--force-billing-fixture') {
    forceBillingFixture = true;
  } else if (!argument.startsWith('-') && outputPath === null) {
    outputPath = argument;
  } else {
    throw new Error(`Unknown or duplicate argument: ${argument}`);
  }
}

if (outputPath === null) {
  throw new Error(
    'Usage: node scripts/make-big-fixture.mjs <output-path> [--intents <count>] [--force-billing-fixture]',
  );
}
if (!Number.isSafeInteger(intentCount) || intentCount < 1 || intentCount > 5000) {
  throw new Error('--intents must be an integer between 1 and 5000');
}

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const resolvedOutput = resolve(outputPath);
const billingFixture = resolve(scriptDirectory, '../public/data.js');
if (resolvedOutput === billingFixture && !forceBillingFixture) {
  throw new Error(
    'Refusing to overwrite public/data.js; pass --force-billing-fixture to make that destructive choice explicit.',
  );
}

const pad = (value) => String(value).padStart(4, '0');
const key = (kind, id) => ({ kind, id });
const edge = (fromKind, fromId, relation, toKind, toId) => ({
  from: key(fromKind, fromId),
  relation,
  to: key(toKind, toId),
});

const notionKinds = ['actor', 'entity', 'value', 'event', 'state'];
const notions = Array.from({ length: 36 }, (_, index) => {
  const number = index + 1;
  const name = `DomainConcept${pad(number)}`;
  const kindName = notionKinds[index % notionKinds.length];
  const definition = `Synthetic ${kindName} ${number} used by the scale fixture.`;
  return {
    name,
    kind: kindName,
    definition,
    canonical: `notion ${name} ${kindName} {\n  def  "${definition}"\n}\n`,
  };
});

const constraintKinds = ['stack', 'architecture', 'quality', 'security', 'convention'];
const constraints = Array.from({ length: 12 }, (_, index) => {
  const number = index + 1;
  const id = `CON-${pad(number)}`;
  const kindName = constraintKinds[index % constraintKinds.length];
  const title = `Synthetic ${kindName} rule ${number}`;
  const scope = index === 0 ? 'global' : `group-${pad(number)}`;
  return {
    id,
    kind: kindName,
    title,
    scope,
    canonical: `constraint ${id} ${kindName} "${title}" {\n  rule  "Keep generated example ${number} coherent."\n  scope ${scope}\n}\n`,
  };
});

const scenarios = [];
const intents = [];
const implementations = [];
const proofs = [];

for (let index = 0; index < intentCount; index += 1) {
  const number = index + 1;
  const intentId = `INT-${pad(number)}`;
  const scenarioId = `SCN-${pad(number)}`;
  const title = `Synthetic intent ${number}`;
  const status = ['active', 'draft', 'deprecated'][index % 3];
  const notionNames = [
    notions[index % notions.length].name,
    notions[(index * 7 + 5) % notions.length].name,
  ];
  const hasScenario = number % 3 === 0;
  const scenarioProofs =
    number % 12 === 0 ? [`tests/scale_${pad(number)}.rs::scenario_${pad(number)}`] : [];
  const scenario = hasScenario
    ? {
        id: scenarioId,
        intent: intentId,
        title: `Synthetic scenario for intent ${number}`,
        notions: notionNames.slice(0, 1),
        proves: scenarioProofs,
      }
    : null;
  if (scenario) scenarios.push(scenario);

  const targetedConstraint = constraints[(index % (constraints.length - 1)) + 1];
  const attachedConstraints =
    number % 5 === 0 ? [constraints[0], targetedConstraint] : [constraints[0]];
  const constraintRefs = attachedConstraints.map((constraint) => ({
    id: constraint.id,
    title: constraint.title,
    scope: constraint.scope,
    canonical: constraint.canonical,
  }));
  const implementationPaths =
    number % 6 === 0 ? [`src/generated/intent_${pad(number)}.rs`] : [];
  intents.push({
    id: intentId,
    title,
    status,
    telos: `Generated behavior ${number} remains deterministic and inspectable.`,
    canonical: `intent ${intentId} "${title}" {\n  status ${status}\n  telos  "Generated behavior ${number} remains deterministic and inspectable."\n}\n`,
    notions: notionNames,
    constraints: constraintRefs,
    implements: implementationPaths,
    scenarios: scenario ? [scenario] : [],
  });

  for (const path of implementationPaths) implementations.push({ path, intent: intentId });
  for (const test of scenarioProofs) proofs.push({ test, scenario: scenarioId });
}

const nodes = [
  ...notions.map((notion) => ({
    key: key('notion', notion.name),
    label: notion.definition,
  })),
  ...intents.map((intent) => ({ key: key('intent', intent.id), label: intent.title })),
  ...scenarios.map((scenario) => ({ key: key('scenario', scenario.id), label: scenario.title })),
  ...constraints.map((constraint) => ({
    key: key('constraint', constraint.id),
    label: constraint.title,
  })),
  ...implementations.map((implementation) => ({
    key: key('code', implementation.path),
    label: implementation.path,
  })),
  ...proofs.map((proof) => ({ key: key('test', proof.test), label: proof.test })),
];

const edges = [];
for (let index = 0; index < intents.length; index += 1) {
  const intent = intents[index];
  const scenario = intent.scenarios[0];

  for (const notion of intent.notions) {
    edges.push(edge('intent', intent.id, 'uses', 'notion', notion));
  }
  if (index > 0 && (index + 1) % 10 === 0) {
    edges.push(edge('intent', intent.id, 'requires', 'intent', intents[index - 1].id));
  }
  if (index >= 5 && (index + 1) % 15 === 0) {
    edges.push(edge('intent', intent.id, 'refines', 'intent', intents[index - 5].id));
  }
  if (index >= 17 && (index + 1) % 30 === 0) {
    edges.push(edge('intent', intent.id, 'excludes', 'intent', intents[index - 17].id));
  }

  const targetedConstraint = intent.constraints[1];
  if (targetedConstraint) {
    edges.push(edge('constraint', targetedConstraint.id, 'constrains', 'intent', intent.id));
  }
  if (scenario) {
    edges.push(edge('scenario', scenario.id, 'verifies', 'intent', intent.id));
    for (const notion of scenario.notions) {
      edges.push(edge('scenario', scenario.id, 'uses', 'notion', notion));
    }
  }
}
for (const implementation of implementations) {
  edges.push(edge('code', implementation.path, 'implements', 'intent', implementation.intent));
}
for (const proof of proofs) {
  edges.push(edge('test', proof.test, 'proves', 'scenario', proof.scenario));
}

const coverageRows = scenarios.flatMap((scenario) =>
  scenario.proves.length === 0
    ? [{ intent: scenario.intent, scenario: scenario.id, test: null }]
    : scenario.proves.map((test) => ({ intent: scenario.intent, scenario: scenario.id, test })),
);

const payload = {
  meta: {
    version: 'scale-fixture',
    build_date: '2026-08-21',
    mode: 'export',
  },
  snapshot: {
    dashboard: {
      state: 'coherent',
      drift: [],
      open_changes: [],
    },
    coverage: {
      notions: notions.length,
      constraints: constraints.length,
      intents_total: intents.length,
      intents_active: intents.filter((intent) => intent.status === 'active').length,
      intents_implemented: intents.filter((intent) => intent.implements.length > 0).length,
      scenarios_total: scenarios.length,
      scenarios_proved: scenarios.filter((scenario) => scenario.proves.length > 0).length,
      rows: coverageRows,
    },
    notions,
    intents,
    scenarios,
    constraints,
    implementations,
    proofs,
    nodes,
    edges,
  },
};

const nodeIds = new Set(nodes.map((node) => `${node.key.kind}:${node.key.id}`));
if (nodeIds.size !== nodes.length) throw new Error('Generated duplicate graph node ids');
for (const graphEdge of edges) {
  const source = `${graphEdge.from.kind}:${graphEdge.from.id}`;
  const target = `${graphEdge.to.kind}:${graphEdge.to.id}`;
  if (!nodeIds.has(source) || !nodeIds.has(target)) {
    throw new Error(`Generated edge with missing endpoint: ${source} -> ${target}`);
  }
}

const output = `window.__TELOS_DATA__ = ${JSON.stringify(payload, null, 2)};\n`;
await mkdir(dirname(resolvedOutput), { recursive: true });
await writeFile(resolvedOutput, output, 'utf8');

console.log(
  `Wrote ${resolvedOutput}: ${intents.length} intents, ${scenarios.length} scenarios, ${nodes.length} nodes, ${edges.length} edges`,
);
