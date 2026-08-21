// Tests for the hand-rolled `.tel` tokenizer (src/tel/tokenizer.ts), written
// before the implementation exists (TDD). The grammar reference is
// crates/telos-core/src/syntax/lexer.rs; these excerpts are copied verbatim
// from real fixtures in demo/billing/telos/ (not read via fs — the tokenizer
// only needs to behave plausibly on realistic input, and inlining keeps this
// test self-contained inside frontend/, independent of the workspace layout
// outside it).
import { describe, expect, it } from 'vitest';

import { tokenize } from './tokenizer';

function reassemble(input: string): string {
  return tokenize(input)
    .map((t) => t.text)
    .join('');
}

function kindsOf(input: string): string[] {
  return tokenize(input).map((t) => t.kind);
}

const INTENT_0042 = `intent INT-0042 "Invoice payment marks it settled" {
  status draft
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
}
`;

const NOTION_INVOICE = `notion Invoice entity {
  def  "A bill issued to a Customer for delivered work."
  attr state   enum(open, settled, cancelled)
  attr balance money
  rel  issued-to -> Customer
}
`;

const CONSTRAINT_0003 = `constraint CON-0003 architecture "Hexagonal boundaries" {
  rule  "Domain code must not import adapter modules."
  scope global
}
`;

const NOTION_CUSTOMER = `notion Customer entity {
  def  "A person or company that receives invoices."
  attr name string
}
`;

describe('tokenize: totality invariant', () => {
  it.each([
    ['empty input', ''],
    ['whitespace-only input', '   \n\t \n  '],
    ['a single newline', '\n'],
    ['an unterminated string', '"unterminated'],
    ['a string with an invalid escape', '"a\\nb"'],
    ['a truncated id literal', 'INT-42'],
    ['a bare unpaired dash', 'a - b'],
    ['exotic unicode', 'emoji 🎉🚀 and accents éàî ñ'],
    ['a real notion excerpt', NOTION_INVOICE],
    ['a real intent excerpt', INTENT_0042],
    ['a real constraint excerpt', CONSTRAINT_0003],
    ['a real second notion excerpt', NOTION_CUSTOMER],
  ])('reproduces the input byte-for-byte for %s', (_label, input) => {
    expect(reassemble(input)).toBe(input);
  });
});

describe('tokenize: token classes', () => {
  it('lexes a plain string literal', () => {
    expect(tokenize('"120.00 EUR"')[0]).toEqual({ kind: 'string', text: '"120.00 EUR"' });
  });

  it('lexes escaped quotes and backslashes inside a string', () => {
    const tokens = tokenize('"a \\"b\\" c\\\\d"');
    expect(tokens[0]).toEqual({ kind: 'string', text: '"a \\"b\\" c\\\\d"' });
  });

  it('lexes an IdLit for each of the four entity prefixes', () => {
    for (const id of ['INT-0042', 'SCN-0107', 'CON-0003', 'CHG-0007']) {
      expect(tokenize(id)[0]).toEqual({ kind: 'idlit', text: id });
    }
  });

  it('lexes a datetime literal', () => {
    expect(tokenize('2026-08-19T12:00:00Z')[0]).toEqual({
      kind: 'date',
      text: '2026-08-19T12:00:00Z',
    });
  });

  it('lexes a date literal', () => {
    expect(tokenize('2026-08-19')[0]).toEqual({ kind: 'date', text: '2026-08-19' });
  });

  it('lexes a decimal literal', () => {
    expect(tokenize('120.50')[0]).toEqual({ kind: 'number', text: '120.50' });
  });

  it('lexes an integer literal', () => {
    expect(tokenize('120')[0]).toEqual({ kind: 'number', text: '120' });
  });

  it('lexes negative decimal and integer literals', () => {
    expect(tokenize('-3.14')[0]).toEqual({ kind: 'number', text: '-3.14' });
    expect(tokenize('-3')[0]).toEqual({ kind: 'number', text: '-3' });
  });

  it('lexes an UpperIdent', () => {
    expect(tokenize('Invoice')[0]).toEqual({ kind: 'ident', text: 'Invoice' });
  });

  it('lexes a lower kebab-case ident', () => {
    expect(tokenize('issued-to')[0]).toEqual({ kind: 'ident', text: 'issued-to' });
  });

  it('disambiguates a dash-arrow from a kebab ident, like the lexer does', () => {
    const tokens = tokenize('issued-to -> Customer').map((t) => [t.kind, t.text]);
    expect(tokens).toEqual([
      ['ident', 'issued-to'],
      ['plain', ' '],
      ['punct', '->'],
      ['plain', ' '],
      ['ident', 'Customer'],
    ]);
  });

  it('lexes every punctuation and operator token', () => {
    const puncts = tokenize('{ } ( ) , : . = == != <= >= < >')
      .filter((t) => t.kind === 'punct')
      .map((t) => t.text);
    expect(puncts).toEqual([
      '{', '}', '(', ')', ',', ':', '.', '=', '==', '!=', '<=', '>=', '<', '>',
    ]);
  });
});

describe('tokenize: keyword heuristic', () => {
  it('marks the first word of a line as a keyword', () => {
    expect(tokenize('rel  issued-to -> Customer')[0]).toEqual({ kind: 'keyword', text: 'rel' });
    expect(tokenize('def  "A bill issued to a Customer."')[0]).toEqual({
      kind: 'keyword',
      text: 'def',
    });
  });

  it('marks the single word right after status/scope/statement as a keyword too', () => {
    expect(kindsOf('status draft')).toEqual(['keyword', 'plain', 'keyword']);
    expect(kindsOf('scope global')).toEqual(['keyword', 'plain', 'keyword']);
    expect(kindsOf('statement event-driven')).toEqual(['keyword', 'plain', 'keyword']);
  });

  it('marks only the single word right after system, not the rest of the line', () => {
    const tokens = tokenize('system shall set Invoice.state = settled');
    expect(tokens[0]).toEqual({ kind: 'keyword', text: 'system' });
    expect(tokens[2]).toEqual({ kind: 'keyword', text: 'shall' });
    expect(tokens[4]).toEqual({ kind: 'ident', text: 'set' });
  });

  it('marks the notion kind word right after the block head name as a keyword', () => {
    const tokens = tokenize('notion Invoice entity {');
    expect(tokens.map((t) => t.kind)).toEqual([
      'keyword', 'plain', 'ident', 'plain', 'keyword', 'plain', 'punct',
    ]);
  });

  it('marks the constraint kind word right after the block head id as a keyword', () => {
    const tokens = tokenize('constraint CON-0003 architecture "Hexagonal boundaries" {');
    expect(tokens[0]).toEqual({ kind: 'keyword', text: 'constraint' });
    expect(tokens[2]).toEqual({ kind: 'idlit', text: 'CON-0003' });
    expect(tokens[4]).toEqual({ kind: 'keyword', text: 'architecture' });
  });

  it('does not chain kind-word treatment onto intent/scenario heads (no kind word there)', () => {
    const tokens = tokenize('intent INT-0042 "Invoice payment marks it settled" {');
    expect(tokens[0]).toEqual({ kind: 'keyword', text: 'intent' });
    expect(tokens[2]).toEqual({ kind: 'idlit', text: 'INT-0042' });
    expect(tokens[4].kind).toBe('string');
  });

  it('treats and/or/not/in/true/false as keywords wherever they appear on a line', () => {
    const tokens = tokenize('when x and y or not z in w true false');
    expect(tokens.find((t) => t.text === 'when')?.kind).toBe('keyword');
    expect(tokens.find((t) => t.text === 'x')?.kind).toBe('ident');
    for (const word of ['and', 'or', 'not', 'in', 'true', 'false']) {
      expect(tokens.find((t) => t.text === word)?.kind).toBe('keyword');
    }
  });

  it('treats the built-in attr type words as keywords wherever they appear', () => {
    const tokens = tokenize('attr balance money');
    expect(tokens.find((t) => t.text === 'balance')?.kind).toBe('ident');
    expect(tokens.find((t) => t.text === 'money')?.kind).toBe('keyword');

    for (const word of ['string', 'int', 'decimal', 'money', 'bool', 'date', 'datetime', 'enum', 'ref']) {
      const found = tokenize(`x: ${word}`).find((t) => t.text === word);
      expect(found?.kind).toBe('keyword');
    }
  });

  it('does not mark an attribute name or an enum value as a keyword', () => {
    const tokens = tokenize('attr state   enum(open, settled, cancelled)');
    expect(tokens.find((t) => t.text === 'state')?.kind).toBe('ident');
    expect(tokens.find((t) => t.text === 'open')?.kind).toBe('ident');
    expect(tokens.find((t) => t.text === 'settled')?.kind).toBe('ident');
  });
});

describe('tokenize: real .tel excerpts', () => {
  const excerpts: Record<string, string> = {
    'the INT-0042 intent': INTENT_0042,
    'the Invoice notion': NOTION_INVOICE,
    'the CON-0003 constraint': CONSTRAINT_0003,
    'the Customer notion': NOTION_CUSTOMER,
  };

  for (const [name, src] of Object.entries(excerpts)) {
    it(`tokenizes ${name} without throwing, reproducing it exactly with plausible classes`, () => {
      const tokens = tokenize(src);
      expect(reassemble(src)).toBe(src);
      const kinds = new Set(tokens.map((t) => t.kind));
      expect(kinds.has('keyword')).toBe(true);
      expect(kinds.has('punct')).toBe(true);
      expect(kinds.has('string')).toBe(true);
    });
  }

  it('classifies id literals and idents plausibly across the intent excerpt', () => {
    const tokens = tokenize(INTENT_0042);
    expect(tokens.some((t) => t.kind === 'idlit' && t.text === 'INT-0042')).toBe(true);
    expect(tokens.some((t) => t.kind === 'idlit' && t.text === 'SCN-0107')).toBe(true);
    expect(tokens.some((t) => t.kind === 'ident' && t.text === 'Invoice')).toBe(true);
  });
});

describe('tokenize: never throws, even on hostile input', () => {
  it.each([
    ['a lone quote', '"'],
    ['a lone backslash', '\\'],
    ['four quotes in a row', '""""'],
    ['three backslashes in a row', '\\\\\\'],
    ['emoji and accents', 'emoji: 🎉🚀👍 and accents: éàî'],
    ['a very long identifier line', 'a'.repeat(50_000)],
    ['a very long run of dashes', '-'.repeat(2_000)],
    ['a very long run of braces', '{'.repeat(1_000) + '}'.repeat(1_000)],
  ])('does not throw on %s', (_label, input) => {
    expect(() => tokenize(input)).not.toThrow();
    expect(reassemble(input)).toBe(input);
  });
});
