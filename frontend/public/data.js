// Dev fixture for `window.__TELOS_DATA__`. Loaded as a classic script by
// index.html, before the Vite entry — see src/data/snapshot.ts.
//
// Hand-built to be structurally identical to what `ViewSnapshot::build()`
// (crates/telos/src/view/model.rs) would serialize: same fields, same
// shapes, same field names (no renames). The content reflects the
// `demo/billing/` corpus — Customer/Invoice/InvoiceIssued/PaymentReceived
// and INT-0017/INT-0042 are taken verbatim from
// demo/billing/telos/{notions,intents}/*.tel — extended with a few more
// notions/intents/scenarios/constraints so every page stub in this task has
// something plausible to eventually render.

// --- notions ----------------------------------------------------------------

const notionCustomer = {
  name: 'Customer',
  kind: 'entity',
  definition: 'A person or company that receives invoices.',
  canonical: `notion Customer entity {
  def  "A person or company that receives invoices."
  attr name string
}
`,
};

const notionInvoice = {
  name: 'Invoice',
  kind: 'entity',
  definition: 'A bill issued to a Customer for delivered work.',
  canonical: `notion Invoice entity {
  def  "A bill issued to a Customer for delivered work."
  attr state   enum(open, settled, cancelled)
  attr balance money
  rel  issued-to -> Customer
}
`,
};

const notionInvoiceIssued = {
  name: 'InvoiceIssued',
  kind: 'event',
  definition: 'An invoice was issued to a customer.',
  canonical: `notion InvoiceIssued event {
  def  "An invoice was issued to a customer."
}
`,
};

const notionPaymentReceived = {
  name: 'PaymentReceived',
  kind: 'event',
  definition: 'A payment arrived for an invoice.',
  canonical: `notion PaymentReceived event {
  def  "A payment arrived for an invoice."
  attr amount money
}
`,
};

const notionBillingClerk = {
  name: 'BillingClerk',
  kind: 'actor',
  definition: 'A staff member who manages customer invoices.',
  canonical: `notion BillingClerk actor {
  def  "A staff member who manages customer invoices."
  attr name string
}
`,
};

const notionDueDatePassed = {
  name: 'DueDatePassed',
  kind: 'event',
  definition: "An invoice's payment due date passed without full payment.",
  canonical: `notion DueDatePassed event {
  def  "An invoice's payment due date passed without full payment."
}
`,
};

const notionInvoiceVoided = {
  name: 'InvoiceVoided',
  kind: 'event',
  definition: 'An invoice was voided by a billing clerk before payment.',
  canonical: `notion InvoiceVoided event {
  def  "An invoice was voided by a billing clerk before payment."
}
`,
};

const notionLateFee = {
  name: 'LateFee',
  kind: 'value',
  definition: 'A flat fee charged when an invoice remains unpaid past its due date.',
  canonical: `notion LateFee value {
  def  "A flat fee charged when an invoice remains unpaid past its due date."
  attr amount money
}
`,
};

const notionOverdue = {
  name: 'Overdue',
  kind: 'state',
  definition: 'An invoice has passed its due date while still unpaid.',
  canonical: `notion Overdue state {
  def  "An invoice has passed its due date while still unpaid."
}
`,
};

const notions = [
  notionBillingClerk,
  notionCustomer,
  notionDueDatePassed,
  notionInvoice,
  notionInvoiceIssued,
  notionInvoiceVoided,
  notionLateFee,
  notionOverdue,
  notionPaymentReceived,
];

// --- constraints --------------------------------------------------------------

const constraintCon0003 = {
  id: 'CON-0003',
  kind: 'architecture',
  title: 'Hexagonal boundaries',
  scope: 'global',
  canonical: `constraint CON-0003 architecture "Hexagonal boundaries" {
  rule  "Domain code must not import adapter modules."
  scope global
}
`,
};

const constraintCon0011 = {
  id: 'CON-0011',
  kind: 'quality',
  title: 'No unchecked unwraps in billing handlers',
  scope: 'INT-0042, INT-0053',
  canonical: `constraint CON-0011 quality "No unchecked unwraps in billing handlers" {
  rule  "Billing request handlers must not call .unwrap() or .expect() on a Result or an Option."
  scope INT-0042, INT-0053
}
`,
};

const constraintCon0019 = {
  id: 'CON-0019',
  kind: 'stack',
  title: 'Money stays fixed-point',
  scope: 'global',
  canonical: `constraint CON-0019 stack "Money stays fixed-point" {
  rule  "Monetary amounts are represented as fixed-point decimals, never as floating point."
  scope global
}
`,
};

const constraints = [constraintCon0003, constraintCon0011, constraintCon0019];

// Applicable to every intent (global scope) vs. only the two CON-0011 targets.
const globalConstraintRefs = [
  { id: constraintCon0003.id, title: constraintCon0003.title, scope: constraintCon0003.scope, canonical: constraintCon0003.canonical },
  { id: constraintCon0019.id, title: constraintCon0019.title, scope: constraintCon0019.scope, canonical: constraintCon0019.canonical },
];
const con0011Ref = { id: constraintCon0011.id, title: constraintCon0011.title, scope: constraintCon0011.scope, canonical: constraintCon0011.canonical };

// --- scenarios (shared between the flat list and each intent's own list) ----

const scn0034 = {
  id: 'SCN-0034',
  intent: 'INT-0005',
  title: 'a new invoice gets a legacy number',
  notions: ['Customer', 'Invoice', 'InvoiceIssued'],
  proves: [],
};

const scn0091 = {
  id: 'SCN-0091',
  intent: 'INT-0017',
  title: 'a newly issued invoice is open',
  notions: ['Customer', 'Invoice', 'InvoiceIssued'],
  proves: [],
};

const scn0107 = {
  id: 'SCN-0107',
  intent: 'INT-0042',
  title: 'full payment settles the invoice',
  notions: ['Invoice', 'PaymentReceived'],
  proves: ['tests/billing.rs::scn_0107_full_payment_settles_the_invoice'],
};

const scn0108 = {
  id: 'SCN-0108',
  intent: 'INT-0042',
  title: 'a payment that exactly zeroes the balance settles the invoice',
  notions: ['Invoice', 'PaymentReceived'],
  proves: [],
};

const scn0142 = {
  id: 'SCN-0142',
  intent: 'INT-0053',
  title: 'a due date passing on an open invoice adds a late fee',
  notions: ['DueDatePassed', 'Invoice'],
  proves: [],
};

const scn0143 = {
  id: 'SCN-0143',
  intent: 'INT-0053',
  title: 'a due date passing on a settled invoice adds no fee',
  notions: ['DueDatePassed', 'Invoice'],
  proves: [],
};

const scn0158 = {
  id: 'SCN-0158',
  intent: 'INT-0061',
  title: 'a clerk voids an open invoice',
  notions: ['BillingClerk', 'Invoice', 'InvoiceVoided'],
  proves: ['tests/billing.rs::scn_0158_a_clerk_voids_an_open_invoice'],
};

const scn0166 = {
  id: 'SCN-0166',
  intent: 'INT-0074',
  title: 'a partial payment lowers the balance without settling the invoice',
  notions: ['Invoice', 'PaymentReceived'],
  proves: [],
};

const scenarios = [scn0034, scn0091, scn0107, scn0108, scn0142, scn0143, scn0158, scn0166];

// --- intents ------------------------------------------------------------------

const intents = [
  {
    id: 'INT-0005',
    title: 'Legacy invoice numbering scheme',
    status: 'deprecated',
    telos: 'Invoice numbers must stay globally unique.',
    canonical: `intent INT-0005 "Legacy invoice numbering scheme" {
  status deprecated
  telos  "Invoice numbers must stay globally unique."
  statement ubiquitous {
    system shall "assign every new invoice the next number in the legacy sequential counter"
  }

  scenario SCN-0034 "a new invoice gets a legacy number" {
    given Customer { name: "ACME" }
    when  InvoiceIssued {}
    then  Invoice.state != cancelled
  }
}
`,
    notions: ['Customer', 'Invoice', 'InvoiceIssued'],
    constraints: globalConstraintRefs,
    implements: [],
    scenarios: [scn0034],
  },
  {
    id: 'INT-0017',
    title: 'Issuing an invoice opens it',
    status: 'draft',
    telos: 'An invoice must start its life open and unpaid.',
    canonical: `intent INT-0017 "Issuing an invoice opens it" {
  status draft
  telos  "An invoice must start its life open and unpaid."
  statement event-driven {
    when   InvoiceIssued on Invoice
    system shall set Invoice.state = open
  }

  scenario SCN-0091 "a newly issued invoice is open" {
    given Customer { name: "ACME" }
    when  InvoiceIssued {}
    then  Invoice.state == open
  }
}
`,
    notions: ['Customer', 'Invoice', 'InvoiceIssued'],
    constraints: globalConstraintRefs,
    implements: [],
    scenarios: [scn0091],
  },
  {
    id: 'INT-0042',
    title: 'Invoice payment marks it settled',
    status: 'active',
    telos: 'Customers must see immediately that their debt is cleared.',
    canonical: `intent INT-0042 "Invoice payment marks it settled" {
  status active
  telos  "Customers must see immediately that their debt is cleared."
  statement event-driven {
    when   PaymentReceived on Invoice
    system shall set Invoice.state = settled
  }
  requires INT-0017

  scenario SCN-0107 "full payment settles the invoice" {
    given Invoice { state: open, balance: "120.00 EUR" }
    when  PaymentReceived { amount: "120.00 EUR" }
    then  Invoice.state == settled
  }

  scenario SCN-0108 "a payment that exactly zeroes the balance settles the invoice" {
    given Invoice { state: open, balance: "42.50 EUR" }
    when  PaymentReceived { amount: "42.50 EUR" }
    then  Invoice.state == settled
  }
}
`,
    notions: ['Invoice', 'PaymentReceived'],
    constraints: [...globalConstraintRefs, con0011Ref],
    implements: ['src/billing/invoice.rs'],
    scenarios: [scn0107, scn0108],
  },
  {
    id: 'INT-0053',
    title: 'Overdue invoices accrue a late fee',
    status: 'active',
    telos: 'Customers must be discouraged from letting invoices go unpaid.',
    canonical: `intent INT-0053 "Overdue invoices accrue a late fee" {
  status active
  telos  "Customers must be discouraged from letting invoices go unpaid."
  statement event-driven {
    when   DueDatePassed on Invoice
    system shall "add the configured late fee to Invoice.balance"
  }

  scenario SCN-0142 "a due date passing on an open invoice adds a late fee" {
    given Invoice { state: open, balance: "80.00 EUR" }
    when  DueDatePassed {}
    then  Invoice.balance != "80.00 EUR"
  }

  scenario SCN-0143 "a due date passing on a settled invoice adds no fee" {
    given Invoice { state: settled, balance: "0.00 EUR" }
    when  DueDatePassed {}
    then  Invoice.balance == "0.00 EUR"
  }
}
`,
    notions: ['DueDatePassed', 'Invoice'],
    constraints: [...globalConstraintRefs, con0011Ref],
    implements: [],
    scenarios: [scn0142, scn0143],
  },
  {
    id: 'INT-0061',
    title: 'Billing clerks can void an unpaid invoice',
    status: 'active',
    telos: 'Clerks must be able to correct a wrongly issued invoice before it is paid.',
    canonical: `intent INT-0061 "Billing clerks can void an unpaid invoice" {
  status active
  telos  "Clerks must be able to correct a wrongly issued invoice before it is paid."
  statement event-driven {
    when   InvoiceVoided on Invoice
    system shall set Invoice.state = cancelled
  }

  scenario SCN-0158 "a clerk voids an open invoice" {
    given BillingClerk { name: "J. Reyes" }
    given Invoice { state: open, balance: "45.00 EUR" }
    when  InvoiceVoided {}
    then  Invoice.state == cancelled
  }
}
`,
    notions: ['BillingClerk', 'Invoice', 'InvoiceVoided'],
    constraints: globalConstraintRefs,
    implements: ['src/billing/clerk.rs'],
    scenarios: [scn0158],
  },
  {
    id: 'INT-0074',
    title: 'Partial payments reduce the invoice balance',
    status: 'draft',
    telos: 'Customers must see their remaining balance update after every partial payment.',
    canonical: `intent INT-0074 "Partial payments reduce the invoice balance" {
  status draft
  telos  "Customers must see their remaining balance update after every partial payment."
  statement event-driven {
    when   PaymentReceived on Invoice
    system shall "reduce Invoice.balance by the amount received"
  }
  requires INT-0042

  scenario SCN-0166 "a partial payment lowers the balance without settling the invoice" {
    given Invoice { state: open, balance: "120.00 EUR" }
    when  PaymentReceived { amount: "50.00 EUR" }
    then  Invoice.state == open
  }
}
`,
    notions: ['Invoice', 'PaymentReceived'],
    constraints: globalConstraintRefs,
    implements: [],
    scenarios: [scn0166],
  },
];

// --- bindings -------------------------------------------------------------

const implementations = [
  { path: 'src/billing/clerk.rs', intent: 'INT-0061' },
  { path: 'src/billing/invoice.rs', intent: 'INT-0042' },
];

const proofs = [
  { test: 'tests/billing.rs::scn_0107_full_payment_settles_the_invoice', scenario: 'SCN-0107' },
  { test: 'tests/billing.rs::scn_0158_a_clerk_voids_an_open_invoice', scenario: 'SCN-0158' },
];

// --- graph (nodes sorted notion < intent < scenario < constraint < code < test,
// edges sorted by (from, relation, to) — the same order ViewSnapshot::build
// produces via its BTreeSet collections) ------------------------------------

const nodes = [
  { key: { kind: 'notion', id: 'BillingClerk' }, label: notionBillingClerk.definition },
  { key: { kind: 'notion', id: 'Customer' }, label: notionCustomer.definition },
  { key: { kind: 'notion', id: 'DueDatePassed' }, label: notionDueDatePassed.definition },
  { key: { kind: 'notion', id: 'Invoice' }, label: notionInvoice.definition },
  { key: { kind: 'notion', id: 'InvoiceIssued' }, label: notionInvoiceIssued.definition },
  { key: { kind: 'notion', id: 'InvoiceVoided' }, label: notionInvoiceVoided.definition },
  { key: { kind: 'notion', id: 'LateFee' }, label: notionLateFee.definition },
  { key: { kind: 'notion', id: 'Overdue' }, label: notionOverdue.definition },
  { key: { kind: 'notion', id: 'PaymentReceived' }, label: notionPaymentReceived.definition },
  { key: { kind: 'intent', id: 'INT-0005' }, label: 'Legacy invoice numbering scheme' },
  { key: { kind: 'intent', id: 'INT-0017' }, label: 'Issuing an invoice opens it' },
  { key: { kind: 'intent', id: 'INT-0042' }, label: 'Invoice payment marks it settled' },
  { key: { kind: 'intent', id: 'INT-0053' }, label: 'Overdue invoices accrue a late fee' },
  { key: { kind: 'intent', id: 'INT-0061' }, label: 'Billing clerks can void an unpaid invoice' },
  { key: { kind: 'intent', id: 'INT-0074' }, label: 'Partial payments reduce the invoice balance' },
  { key: { kind: 'scenario', id: 'SCN-0034' }, label: scn0034.title },
  { key: { kind: 'scenario', id: 'SCN-0091' }, label: scn0091.title },
  { key: { kind: 'scenario', id: 'SCN-0107' }, label: scn0107.title },
  { key: { kind: 'scenario', id: 'SCN-0108' }, label: scn0108.title },
  { key: { kind: 'scenario', id: 'SCN-0142' }, label: scn0142.title },
  { key: { kind: 'scenario', id: 'SCN-0143' }, label: scn0143.title },
  { key: { kind: 'scenario', id: 'SCN-0158' }, label: scn0158.title },
  { key: { kind: 'scenario', id: 'SCN-0166' }, label: scn0166.title },
  { key: { kind: 'constraint', id: 'CON-0003' }, label: constraintCon0003.title },
  { key: { kind: 'constraint', id: 'CON-0011' }, label: constraintCon0011.title },
  { key: { kind: 'constraint', id: 'CON-0019' }, label: constraintCon0019.title },
  { key: { kind: 'code', id: 'src/billing/clerk.rs' }, label: 'src/billing/clerk.rs' },
  { key: { kind: 'code', id: 'src/billing/invoice.rs' }, label: 'src/billing/invoice.rs' },
  { key: { kind: 'test', id: 'tests/billing.rs::scn_0107_full_payment_settles_the_invoice' }, label: 'tests/billing.rs::scn_0107_full_payment_settles_the_invoice' },
  { key: { kind: 'test', id: 'tests/billing.rs::scn_0158_a_clerk_voids_an_open_invoice' }, label: 'tests/billing.rs::scn_0158_a_clerk_voids_an_open_invoice' },
];

function edge(fromKind, fromId, relation, toKind, toId) {
  return { from: { kind: fromKind, id: fromId }, relation, to: { kind: toKind, id: toId } };
}

const edges = [
  // intents
  edge('intent', 'INT-0017', 'uses', 'notion', 'Invoice'),
  edge('intent', 'INT-0017', 'uses', 'notion', 'InvoiceIssued'),
  edge('intent', 'INT-0042', 'requires', 'intent', 'INT-0017'),
  edge('intent', 'INT-0042', 'uses', 'notion', 'Invoice'),
  edge('intent', 'INT-0042', 'uses', 'notion', 'PaymentReceived'),
  edge('intent', 'INT-0053', 'uses', 'notion', 'DueDatePassed'),
  edge('intent', 'INT-0053', 'uses', 'notion', 'Invoice'),
  edge('intent', 'INT-0061', 'uses', 'notion', 'Invoice'),
  edge('intent', 'INT-0061', 'uses', 'notion', 'InvoiceVoided'),
  edge('intent', 'INT-0074', 'requires', 'intent', 'INT-0042'),
  edge('intent', 'INT-0074', 'uses', 'notion', 'Invoice'),
  edge('intent', 'INT-0074', 'uses', 'notion', 'PaymentReceived'),
  // scenarios
  edge('scenario', 'SCN-0034', 'verifies', 'intent', 'INT-0005'),
  edge('scenario', 'SCN-0034', 'uses', 'notion', 'Customer'),
  edge('scenario', 'SCN-0034', 'uses', 'notion', 'Invoice'),
  edge('scenario', 'SCN-0034', 'uses', 'notion', 'InvoiceIssued'),
  edge('scenario', 'SCN-0091', 'verifies', 'intent', 'INT-0017'),
  edge('scenario', 'SCN-0091', 'uses', 'notion', 'Customer'),
  edge('scenario', 'SCN-0091', 'uses', 'notion', 'Invoice'),
  edge('scenario', 'SCN-0091', 'uses', 'notion', 'InvoiceIssued'),
  edge('scenario', 'SCN-0107', 'verifies', 'intent', 'INT-0042'),
  edge('scenario', 'SCN-0107', 'uses', 'notion', 'Invoice'),
  edge('scenario', 'SCN-0107', 'uses', 'notion', 'PaymentReceived'),
  edge('scenario', 'SCN-0108', 'verifies', 'intent', 'INT-0042'),
  edge('scenario', 'SCN-0108', 'uses', 'notion', 'Invoice'),
  edge('scenario', 'SCN-0108', 'uses', 'notion', 'PaymentReceived'),
  edge('scenario', 'SCN-0142', 'verifies', 'intent', 'INT-0053'),
  edge('scenario', 'SCN-0142', 'uses', 'notion', 'DueDatePassed'),
  edge('scenario', 'SCN-0142', 'uses', 'notion', 'Invoice'),
  edge('scenario', 'SCN-0143', 'verifies', 'intent', 'INT-0053'),
  edge('scenario', 'SCN-0143', 'uses', 'notion', 'DueDatePassed'),
  edge('scenario', 'SCN-0143', 'uses', 'notion', 'Invoice'),
  edge('scenario', 'SCN-0158', 'verifies', 'intent', 'INT-0061'),
  edge('scenario', 'SCN-0158', 'uses', 'notion', 'BillingClerk'),
  edge('scenario', 'SCN-0158', 'uses', 'notion', 'Invoice'),
  edge('scenario', 'SCN-0158', 'uses', 'notion', 'InvoiceVoided'),
  edge('scenario', 'SCN-0166', 'verifies', 'intent', 'INT-0074'),
  edge('scenario', 'SCN-0166', 'uses', 'notion', 'Invoice'),
  edge('scenario', 'SCN-0166', 'uses', 'notion', 'PaymentReceived'),
  // constraints (only scope-specific constraints get a `constrains` edge —
  // global ones apply everywhere without one, matching
  // ViewSnapshot::build/model.rs)
  edge('constraint', 'CON-0011', 'constrains', 'intent', 'INT-0042'),
  edge('constraint', 'CON-0011', 'constrains', 'intent', 'INT-0053'),
  // bindings
  edge('code', 'src/billing/clerk.rs', 'implements', 'intent', 'INT-0061'),
  edge('code', 'src/billing/invoice.rs', 'implements', 'intent', 'INT-0042'),
  edge('test', 'tests/billing.rs::scn_0107_full_payment_settles_the_invoice', 'proves', 'scenario', 'SCN-0107'),
  edge('test', 'tests/billing.rs::scn_0158_a_clerk_voids_an_open_invoice', 'proves', 'scenario', 'SCN-0158'),
];

// --- coverage -------------------------------------------------------------

const coverage = {
  notions: notions.length,
  constraints: constraints.length,
  intents_total: intents.length,
  intents_active: intents.filter((intent) => intent.status === 'active').length,
  intents_implemented: intents.filter((intent) => intent.implements.length > 0).length,
  scenarios_total: scenarios.length,
  scenarios_proved: scenarios.filter((scenario) => scenario.proves.length > 0).length,
  rows: scenarios.flatMap((scenario) =>
    scenario.proves.length === 0
      ? [{ intent: scenario.intent, scenario: scenario.id, test: null }]
      : scenario.proves.map((test) => ({ intent: scenario.intent, scenario: scenario.id, test })),
  ),
};

// --- dashboard --------------------------------------------------------------

const dashboard = {
  state: 'changing',
  drift: [],
  open_changes: [
    {
      id: 'CHG-0007',
      status: 'implementing',
      obligations: ['prove SCN-0142', 'prove SCN-0143'],
    },
  ],
};

// --- payload ----------------------------------------------------------------

window.__TELOS_DATA__ = {
  meta: {
    version: '0.8.2',
    build_date: '2026-08-21',
    mode: 'live',
  },
  snapshot: {
    dashboard,
    coverage,
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
