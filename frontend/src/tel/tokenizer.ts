export type TelTokenKind =
  | 'keyword'
  | 'ident'
  | 'idlit'
  | 'string'
  | 'number'
  | 'date'
  | 'punct'
  | 'plain';

export interface TelToken {
  kind: TelTokenKind;
  text: string;
}

const TOKEN_PATTERNS: ReadonlyArray<[TelTokenKind, RegExp]> = [
  ['string', /^"(?:[^"\\\n]|\\["\\])*"/],
  ['idlit', /^(?:INT|SCN|CON|CHG)-\d{4,}/],
  ['date', /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ?/],
  ['date', /^\d{4}-\d\d-\d\d/],
  ['number', /^-?\d+\.\d+/],
  ['number', /^-?\d+/],
  ['ident', /^[A-Z][A-Za-z0-9]*/],
  ['ident', /^[a-z][a-z0-9_]*(?:-[a-z0-9][a-z0-9_]*)*/],
  ['punct', /^(?:->|==|!=|<=|>=|[{}(),:.=<>])/],
];

const EVERYWHERE_KEYWORDS = new Set([
  'and',
  'or',
  'not',
  'in',
  'true',
  'false',
  'string',
  'int',
  'decimal',
  'money',
  'bool',
  'date',
  'datetime',
  'enum',
  'ref',
]);

const LINE_KEYWORDS = new Set([
  'notion',
  'intent',
  'constraint',
  'def',
  'attr',
  'rel',
  'refines',
  'requires',
  'excludes',
  'status',
  'telos',
  'statement',
  'when',
  'while',
  'if',
  'where',
  'system',
  'scenario',
  'given',
  'then',
  'check',
  'rule',
  'scope',
]);

const FOLLOWING_KEYWORDS = new Set(['system', 'status', 'statement', 'scope']);
const BLOCK_HEADS = new Set(['notion', 'constraint']);

function isWord(token: TelToken): boolean {
  return token.kind === 'ident' || token.kind === 'idlit';
}

function classifyWord(token: TelToken, wordsOnLine: TelToken[]): TelTokenKind {
  if (token.kind !== 'ident') return token.kind;

  if (EVERYWHERE_KEYWORDS.has(token.text)) return 'keyword';
  if (wordsOnLine.length === 0 && LINE_KEYWORDS.has(token.text)) return 'keyword';

  const previous = wordsOnLine.at(-1);
  if (previous?.kind === 'keyword' && FOLLOWING_KEYWORDS.has(previous.text)) return 'keyword';

  const head = wordsOnLine[0];
  if (head?.kind === 'keyword' && BLOCK_HEADS.has(head.text) && wordsOnLine.length === 2) {
    return 'keyword';
  }

  return 'ident';
}

/**
 * Produces display tokens for canonical `.tel` source. Unlike the Rust lexer,
 * this renderer-oriented tokenizer always recovers by emitting plain text.
 */
export function tokenize(src: string): TelToken[] {
  const tokens: TelToken[] = [];
  let index = 0;
  let wordsOnLine: TelToken[] = [];

  while (index < src.length) {
    const rest = src.slice(index);
    let matched: TelToken | undefined;

    for (const [kind, pattern] of TOKEN_PATTERNS) {
      const text = pattern.exec(rest)?.[0];
      if (text) {
        matched = { kind, text };
        break;
      }
    }

    const token = matched ?? { kind: 'plain' as const, text: rest[0] };
    if (isWord(token)) {
      token.kind = classifyWord(token, wordsOnLine);
      wordsOnLine.push(token);
    }

    tokens.push(token);
    index += token.text.length;
    if (token.text.includes('\n')) wordsOnLine = [];
  }

  return tokens;
}
