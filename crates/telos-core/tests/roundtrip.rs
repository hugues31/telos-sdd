//! Byte-exact round-trip suite for the canonical `.tel` emitter.
//!
//! Three angles, all of them checking the same invariant from a different
//! side:
//!
//! 1. **Corpus idempotence** -- for every `.tel` file of the Billing fixture
//!    tree, `emit_file(&parse(bytes)) == bytes`, compared byte for byte.
//!    This is the strongest statement of the canonical form: the corpus
//!    files *are* the specification of the emitter's output, so padding,
//!    blank lines, indentation and ordering are all pinned at once.
//! 2. **Programmatic round-trip** -- an `Intent` built in code (spans
//!    `Default`, i.e. no source to point at) emitted, re-parsed and
//!    re-emitted must yield the same string: the emitter's output is always
//!    parseable, and parsing loses nothing the emitter puts back.
//! 3. **Normalization** -- the emitter is not a pretty-printer of what was
//!    written but of what was *meant*: redundant parentheses disappear, and
//!    relation ids, scenarios and bindings come out sorted whatever order
//!    they were built in.

use std::fs;
use std::path::{Path, PathBuf};

use telos_core::emit::{
    emit_bindings, emit_constraint, emit_expr, emit_file, emit_intent, emit_literal,
};
use telos_core::ids::{ConstraintId, FieldName, IntentId, NotionName, RepoPath, ScenarioId};
use telos_core::model::{
    Action, AttrRef, Binding, CmpOp, Constraint, ConstraintKind, Expr, InstanceStep, Intent,
    IntentStatus, Literal, Operand, Rule, Scenario, Scope, Statement, TelFile, TestRef,
};
use telos_core::span::{Sp, Span};
use telos_core::syntax::{
    parse_bindings_file, parse_constraint_file, parse_expr, parse_intent_file, parse_notion_file,
};

// --- corpus helpers ------------------------------------------------------

/// The JSON payload fixture tree, copied into throwaway Git repositories.
fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/billing")
}

/// Every `.tel` file of the corpus, as repo-relative paths, sorted so the
/// suite reports the same order everywhere.
fn corpus_tel_files() -> Vec<String> {
    let mut found = Vec::new();
    collect_tel(&corpus_root(), &mut found);
    found.sort();
    found
}

fn collect_tel(dir: &Path, out: &mut Vec<String>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_tel(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "tel") {
            let rel = path.strip_prefix(corpus_root()).expect("under the corpus");
            // `.tel` paths are repo-relative with `/` separators, on every OS.
            out.push(
                rel.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
}

/// Reads a corpus file as raw bytes turned into a `String` -- deliberately
/// not `fs::read_to_string` with any normalization: the test is about bytes.
fn read_corpus(rel: &str) -> String {
    let path = corpus_root().join(rel);
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    String::from_utf8(bytes).unwrap_or_else(|e| panic!("{rel} is not UTF-8: {e}"))
}

/// Parses a corpus file with the rule its location calls for:
/// one file = one entity, `bindings.tel` excepted.
fn parse_corpus(rel: &str, src: &str) -> TelFile {
    /// A parse failure in a fixture is a bug in the fixture: report which
    /// file, and stop.
    fn ok<T, E>(rel: &str, result: Result<T, Vec<E>>) -> T {
        match result {
            Ok(node) => node,
            Err(diags) => panic!("{rel} must parse, got {} diagnostic(s)", diags.len()),
        }
    }
    let path = RepoPath::new(rel);
    if rel.ends_with("bindings.tel") {
        TelFile::Bindings(ok(rel, parse_bindings_file(&path, src)))
    } else if rel.contains("notions/") {
        TelFile::Notion(ok(rel, parse_notion_file(&path, src)))
    } else if rel.contains("intents/") {
        TelFile::Intent(ok(rel, parse_intent_file(&path, src)))
    } else if rel.contains("constraints/") {
        TelFile::Constraint(ok(rel, parse_constraint_file(&path, src)))
    } else {
        panic!("{rel}: no parse rule for this location");
    }
}

// --- 1. corpus idempotence ----------------------------------------------

#[test]
fn corpus_holds_exactly_the_payload_schema_tel_files() {
    // Guards the suite below: were a corpus file to go missing, its
    // round-trip assertion would silently stop running.
    assert_eq!(
        corpus_tel_files(),
        vec![
            "telos/bindings.tel",
            "telos/constraints/CON-0003.tel",
            "telos/intents/INT-0017.tel",
            "telos/intents/INT-0042.tel",
            "telos/notions/Customer.tel",
            "telos/notions/Invoice.tel",
            "telos/notions/InvoiceIssued.tel",
            "telos/notions/PaymentReceived.tel",
        ]
    );
}

#[test]
fn every_corpus_tel_file_round_trips_byte_exact() {
    for rel in corpus_tel_files() {
        let src = read_corpus(&rel);
        let emitted = emit_file(&parse_corpus(&rel, &src));
        assert_eq!(emitted, src, "{rel} is not byte-exact after a round-trip");
    }
}

#[test]
fn corpus_tel_files_are_lf_only_with_one_trailing_newline() {
    // The canonical form is a byte-level contract. A checkout
    // that rewrote line endings would make the assertions above vacuous, so
    // the file bytes are checked head-on too -- this is what the repo's
    // `.gitattributes` (`*.tel text eol=lf`) protects on Windows.
    for rel in corpus_tel_files() {
        let src = read_corpus(&rel);
        assert!(!src.contains('\r'), "{rel} contains a CR");
        assert!(src.ends_with('\n'), "{rel} has no trailing newline");
        assert!(!src.ends_with("\n\n"), "{rel} has a blank line at the end");
        for (n, line) in src.lines().enumerate() {
            assert!(
                line.trim_end() == line,
                "{rel}:{} has trailing whitespace",
                n + 1
            );
        }
    }
}

#[test]
fn invoice_notion_is_byte_exact_down_to_its_padding() {
    // The reference file for round-trip guarantees, spelled out: keyword padding (`def  `,
    // `attr `, `rel  `), attr-name padding (`state` to `balance`'s width),
    // enum rendering, 2-space indent.
    let src = read_corpus("telos/notions/Invoice.tel");
    assert_eq!(
        src,
        concat!(
            "notion Invoice entity {\n",
            "  def  \"A bill issued to a Customer for delivered work.\"\n",
            "  attr state   enum(open, settled, cancelled)\n",
            "  attr balance money\n",
            "  rel  issued-to -> Customer\n",
            "}\n",
        )
    );
    assert_eq!(
        emit_file(&parse_corpus("telos/notions/Invoice.tel", &src)),
        src
    );
}

// --- 2. programmatic round-trip -----------------------------------------

fn notion(name: &str) -> Sp<NotionName> {
    Sp {
        node: NotionName::new(name).unwrap(),
        span: Span::default(),
    }
}

fn field(name: &str) -> Sp<FieldName> {
    Sp {
        node: FieldName::new(name).unwrap(),
        span: Span::default(),
    }
}

fn intent_id(n: u32) -> Sp<IntentId> {
    Sp {
        node: IntentId(n),
        span: Span::default(),
    }
}

fn symbol(s: &str) -> Literal {
    Literal::Symbol(Sp {
        node: s.to_string(),
        span: Span::default(),
    })
}

fn attr_ref(n: &str, f: &str) -> AttrRef {
    AttrRef {
        notion: notion(n),
        attr: field(f),
    }
}

fn eq(n: &str, f: &str, value: Literal) -> Expr {
    Expr::Cmp {
        op: CmpOp::Eq,
        lhs: Operand::Ref(attr_ref(n, f)),
        rhs: Operand::Lit(value),
    }
}

/// A minimal well-formed scenario, used where only its id matters.
fn scenario(id: u32, title: &str) -> Scenario {
    Scenario {
        id: ScenarioId(id),
        title: title.to_string(),
        given: vec![InstanceStep {
            notion: notion("Invoice"),
            fields: vec![(field("state"), symbol("open"))],
        }],
        when: InstanceStep {
            notion: notion("PaymentReceived"),
            fields: vec![],
        },
        then: vec![eq("Invoice", "state", symbol("settled"))],
    }
}

/// `SCN-0107` of `telos/intents/INT-0042.tel`, rebuilt in code.
fn corpus_scenario() -> Scenario {
    Scenario {
        id: ScenarioId(107),
        title: "full payment settles the invoice".to_string(),
        given: vec![InstanceStep {
            notion: notion("Invoice"),
            fields: vec![
                (field("state"), symbol("open")),
                (field("balance"), Literal::Str("120.00 EUR".to_string())),
            ],
        }],
        when: InstanceStep {
            notion: notion("PaymentReceived"),
            fields: vec![(field("amount"), Literal::Str("120.00 EUR".to_string()))],
        },
        then: vec![eq("Invoice", "state", symbol("settled"))],
    }
}

/// An intent built entirely in code, with `Default` spans: nothing here ever
/// came from a source file, so nothing can be echoed back from one. It is
/// `telos/intents/INT-0042.tel`, to the byte.
fn programmatic_intent() -> Intent {
    Intent {
        id: IntentId(42),
        title: "Invoice payment marks it settled".to_string(),
        status: IntentStatus::Active,
        telos: "Customers must see immediately that their debt is cleared.".to_string(),
        statement: Statement::EventDriven {
            event: notion("PaymentReceived"),
            on: Some(notion("Invoice")),
            action: Action::Set {
                target: attr_ref("Invoice", "state"),
                value: symbol("settled"),
            },
        },
        refines: vec![],
        requires: vec![intent_id(17)],
        excludes: vec![],
        scenarios: vec![corpus_scenario()],
    }
}

#[test]
fn programmatic_intent_survives_emit_parse_emit() {
    let once = emit_intent(&programmatic_intent());
    let reparsed = parse_intent_file(&RepoPath::new("telos/intents/INT-0042.tel"), &once)
        .expect("the emitter's output must parse");
    let twice = emit_intent(&reparsed);
    assert_eq!(once, twice);
}

#[test]
fn programmatic_intent_emits_the_corpus_bytes() {
    // Same intent as `telos/intents/INT-0042.tel`: an in-code model and a
    // parsed one must be indistinguishable once emitted.
    assert_eq!(
        emit_intent(&programmatic_intent()),
        read_corpus("telos/intents/INT-0042.tel")
    );
}

#[test]
fn every_corpus_file_survives_a_second_round_trip() {
    for rel in corpus_tel_files() {
        let src = read_corpus(&rel);
        let once = emit_file(&parse_corpus(&rel, &src));
        let twice = emit_file(&parse_corpus(&rel, &once));
        assert_eq!(once, twice, "{rel} is not stable on a second round-trip");
    }
}

// --- 3a. minimal parentheses --------------------------------------------

fn expr(src: &str) -> Expr {
    parse_expr(src).unwrap_or_else(|d| panic!("`{src}` must parse: {}", d.message))
}

#[test]
fn redundant_parentheses_are_dropped() {
    assert_eq!(
        emit_expr(&expr("(Invoice.state == 1) and Customer.name == 2")),
        "Invoice.state == 1 and Customer.name == 2"
    );
}

#[test]
fn parentheses_are_emitted_only_where_precedence_demands_them() {
    // `or` < `and` < `not` < comparison: a child binds looser than its
    // parent exactly when it needs to be parenthesized.
    let cases = [
        // an `or` under an `and` must keep its parentheses...
        (
            "(Invoice.state == open or Invoice.state == draft) and Customer.name == \"ACME\"",
            "(Invoice.state == open or Invoice.state == draft) and Customer.name == \"ACME\"",
        ),
        // ...but an `and` under an `or` never needs any.
        (
            "Invoice.state == open or (Invoice.state == draft and Customer.name == \"ACME\")",
            "Invoice.state == open or Invoice.state == draft and Customer.name == \"ACME\"",
        ),
        // `not` binds tighter than `and`, so a conjunction under it stays
        // parenthesized...
        (
            "not (Invoice.state == open and Customer.name == \"ACME\")",
            "not (Invoice.state == open and Customer.name == \"ACME\")",
        ),
        // ...while a negated comparison beside a conjunct needs nothing.
        (
            "(not Invoice.state == open) and Customer.name == \"ACME\"",
            "not Invoice.state == open and Customer.name == \"ACME\"",
        ),
        // A comparison is a leaf: never parenthesized.
        ("(Invoice.state == open)", "Invoice.state == open"),
        // `in` is a leaf too.
        (
            "not (Invoice.state in (open, draft))",
            "not Invoice.state in (open, draft)",
        ),
        // Nesting on the right of a same-precedence operator re-associates
        // to the left, which prints identically either way.
        (
            "Invoice.state == a or (Invoice.state == b or Invoice.state == c)",
            "Invoice.state == a or Invoice.state == b or Invoice.state == c",
        ),
    ];
    for (src, expected) in cases {
        let emitted = emit_expr(&expr(src));
        assert_eq!(emitted, expected, "for `{src}`");
        // Whatever the emitter drops, it must not need back: the output is
        // a fixed point of parse-then-emit, associativity included.
        assert_eq!(emit_expr(&expr(&emitted)), emitted, "for `{src}`");
    }
}

#[test]
fn every_comparison_operator_round_trips() {
    for src in [
        "Invoice.balance == 1",
        "Invoice.balance != 1",
        "Invoice.balance < 1",
        "Invoice.balance <= 1",
        "Invoice.balance > 1",
        "Invoice.balance >= 1",
        "Invoice.balance in (1, 2, 3)",
        "1 == Invoice.balance",
    ] {
        assert_eq!(emit_expr(&expr(src)), src);
    }
}

#[test]
fn literals_re_emit_their_preserved_lexeme() {
    // A decimal is never routed through a float, and dates
    // keep the exact lexeme they were written with.
    assert_eq!(
        emit_literal(&Literal::Decimal("120.50".to_string())),
        "120.50"
    );
    assert_eq!(
        emit_literal(&Literal::Decimal("-0.10".to_string())),
        "-0.10"
    );
    assert_eq!(
        emit_literal(&Literal::Date("2026-01-31".to_string())),
        "2026-01-31"
    );
    assert_eq!(
        emit_literal(&Literal::Datetime("2026-01-31T12:00:00Z".to_string())),
        "2026-01-31T12:00:00Z"
    );
    assert_eq!(emit_literal(&Literal::Int(-42)), "-42");
    assert_eq!(emit_literal(&Literal::Bool(true)), "true");
    assert_eq!(emit_literal(&Literal::Bool(false)), "false");
    assert_eq!(emit_literal(&symbol("settled")), "settled");
    // Only `\"` and `\\` are escaped.
    assert_eq!(
        emit_literal(&Literal::Str("a \"b\" \\ c".to_string())),
        "\"a \\\"b\\\" \\\\ c\""
    );
    // A money amount is a plain string lexeme (no `Money` literal variant).
    assert_eq!(
        emit_literal(&Literal::Str("120.00 EUR".to_string())),
        "\"120.00 EUR\""
    );
}

#[test]
fn escaped_strings_round_trip_through_the_parser() {
    let src = "Invoice.state == \"a \\\"b\\\" \\\\ c\"";
    assert_eq!(emit_expr(&expr(src)), src);
}

// --- 3b. sorting on emit -------------------------------------------------

/// The programmatic intent with its relation lists and scenarios supplied
/// deliberately out of order.
fn unsorted_intent() -> Intent {
    Intent {
        requires: vec![intent_id(20), intent_id(3)],
        refines: vec![intent_id(9), intent_id(1)],
        excludes: vec![intent_id(300), intent_id(30)],
        scenarios: vec![scenario(107, "later"), scenario(91, "earlier")],
        ..programmatic_intent()
    }
}

#[test]
fn relation_lines_are_sorted_ascending_on_emit() {
    let emitted = emit_intent(&unsorted_intent());
    let relations: Vec<&str> = emitted
        .lines()
        .map(str::trim)
        .filter(|l| {
            l.starts_with("refines") || l.starts_with("requires") || l.starts_with("excludes")
        })
        .collect();
    assert_eq!(
        relations,
        vec![
            // Grammar order between the groups...
            "refines INT-0001",
            "refines INT-0009",
            "requires INT-0003",
            "requires INT-0020",
            "excludes INT-0030",
            "excludes INT-0300",
        ]
    );
    // ...and no keyword padding on relation lines: a single space.
    assert!(emitted.contains("\n  requires INT-0003\n"));
}

#[test]
fn scenarios_are_sorted_ascending_on_emit() {
    let emitted = emit_intent(&unsorted_intent());
    let first = emitted.find("SCN-0091").expect("SCN-0091 is emitted");
    let second = emitted.find("SCN-0107").expect("SCN-0107 is emitted");
    assert!(first < second, "scenarios must be emitted in id order");
}

#[test]
fn scenarios_are_separated_by_exactly_one_blank_line() {
    // One blank line before each scenario, nowhere else.
    let emitted = emit_intent(&unsorted_intent());
    assert!(!emitted.contains("\n\n\n"), "no double blank line");
    assert_eq!(
        emitted.matches("\n\n").count(),
        2,
        "one before each scenario"
    );
    assert!(emitted.contains("\n  excludes INT-0300\n\n  scenario SCN-0091 "));
}

fn implements(path: &str, id: u32) -> Binding {
    Binding::Implements {
        path: RepoPath::new(path),
        intent: intent_id(id),
    }
}

fn proves(test: &str, id: u32) -> Binding {
    Binding::Proves {
        test: test.parse::<TestRef>().unwrap(),
        scenario: Sp {
            node: ScenarioId(id),
            span: Span::default(),
        },
    }
}

#[test]
fn bindings_are_grouped_and_sorted_on_emit() {
    // Every `implements` first, sorted; then every `proves`.
    let emitted = emit_bindings(&[
        proves("tests/z.rs::b", 2),
        implements("src/z.rs", 20),
        proves("tests/a.rs::a", 1),
        implements("src/a.rs", 3),
        implements("src/a.rs", 1),
    ]);
    assert_eq!(
        emitted,
        concat!(
            "implements \"src/a.rs\" -> INT-0001\n",
            "implements \"src/a.rs\" -> INT-0003\n",
            "implements \"src/z.rs\" -> INT-0020\n",
            "proves     \"tests/a.rs::a\" -> SCN-0001\n",
            "proves     \"tests/z.rs::b\" -> SCN-0002\n",
        )
    );
}

#[test]
fn an_empty_bindings_file_emits_nothing() {
    // `bindings-file = { binding-line }` -- an empty file is valid, and an
    // empty file is zero bytes, not a lone newline.
    assert_eq!(emit_bindings(&[]), "");
    assert_eq!(emit_file(&TelFile::Bindings(vec![])), "");
    let parsed = parse_bindings_file(&RepoPath::new("telos/bindings.tel"), "").unwrap();
    assert_eq!(emit_bindings(&parsed), "");
}

#[test]
fn a_proves_binding_without_a_test_name_emits_the_bare_path() {
    assert_eq!(
        emit_bindings(&[proves("tests/billing.rs", 107)]),
        "proves     \"tests/billing.rs\" -> SCN-0107\n"
    );
}

fn unsorted_constraint() -> Constraint {
    Constraint {
        id: ConstraintId(1),
        kind: ConstraintKind::Quality,
        title: "Balances stay positive".to_string(),
        rule: Rule::Text("Invoices never go negative.".to_string()),
        scope: Scope::Intents(vec![intent_id(42), intent_id(17)]),
        check: None,
    }
}

#[test]
fn constraint_scope_intents_are_sorted_ascending_on_emit() {
    // `scope`'s intent list is sorted, not echoed in model order.
    let emitted = emit_constraint(&unsorted_constraint());
    assert!(emitted.contains("\n  scope INT-0017, INT-0042\n"));
}

#[test]
fn constraint_with_unsorted_scope_survives_emit_parse_emit() {
    let emitted = emit_file(&TelFile::Constraint(unsorted_constraint()));
    let reparsed = parse_constraint_file(&RepoPath::new("telos/constraints/X.tel"), &emitted)
        .unwrap_or_else(|d| panic!("must parse:\n{emitted}\n({} diagnostics)", d.len()));
    assert_eq!(
        emit_file(&TelFile::Constraint(reparsed)),
        emitted,
        "emit -> parse -> emit must be a fixed point"
    );
}

// --- coverage of the shapes the corpus does not exercise ----------------

/// Emits `body` as the sole `statement` block of an otherwise fixed intent,
/// then checks the whole file survives a parse.
fn intent_file_with(statement: Statement) -> String {
    let intent = Intent {
        statement,
        requires: vec![],
        scenarios: vec![],
        ..programmatic_intent()
    };
    let emitted = emit_intent(&intent);
    let reparsed = parse_intent_file(&RepoPath::new("telos/intents/INT-0042.tel"), &emitted)
        .unwrap_or_else(|d| panic!("must parse:\n{emitted}\n({} diagnostics)", d.len()));
    assert_eq!(emit_intent(&reparsed), emitted);
    emitted
}

#[test]
fn every_statement_template_pads_its_body_keyword_to_six() {
    let free = Action::Free("notify the auditor".to_string());
    assert!(
        intent_file_with(Statement::Ubiquitous {
            action: free.clone()
        })
        .contains("  statement ubiquitous {\n    system shall \"notify the auditor\"\n  }\n")
    );
    assert!(
        intent_file_with(Statement::EventDriven {
            event: notion("PaymentReceived"),
            on: None,
            action: free.clone(),
        })
        .contains("    when   PaymentReceived\n    system shall ")
    );
    assert!(
        intent_file_with(Statement::StateDriven {
            subject: attr_ref("Invoice", "state"),
            value: symbol("open"),
            action: free.clone(),
        })
        .contains("    while  Invoice.state == open\n    system shall ")
    );
    assert!(
        intent_file_with(Statement::Unwanted {
            condition: eq("Invoice", "balance", Literal::Int(0)),
            action: free.clone(),
        })
        .contains("    if     Invoice.balance == 0\n    system shall ")
    );
    assert!(
        intent_file_with(Statement::Optional {
            feature: FieldName::new("late-fees").unwrap(),
            action: free,
        })
        .contains("    where  late-fees\n    system shall ")
    );
}

#[test]
fn an_intent_without_scenarios_ends_right_after_its_header() {
    let emitted = intent_file_with(Statement::Ubiquitous {
        action: Action::Free("track invoices".to_string()),
    });
    assert!(!emitted.contains("\n\n"), "no stray blank line:\n{emitted}");
    assert!(emitted.ends_with("  }\n}\n"));
}

#[test]
fn shapes_the_corpus_does_not_cover_round_trip_through_the_parser() {
    // Each source below is written in canonical form by hand; parsing then
    // emitting it must give the exact same bytes back.
    let notions = [
        // Every scalar attr type, a `ref(...)`, several rels: the attr-name
        // and rel-name paddings are independent.
        concat!(
            "notion Ledger value {\n",
            "  def  \"Everything an attribute can be.\"\n",
            "  attr a         string\n",
            "  attr bb        int\n",
            "  attr ccc       decimal\n",
            "  attr dddd      money\n",
            "  attr eeeee     bool\n",
            "  attr ffffff    date\n",
            "  attr ggggggg   datetime\n",
            "  attr hhhhhhhh  enum(one, two)\n",
            "  attr iiiiiiiii ref(Invoice)\n",
            "  rel  x  -> Invoice\n",
            "  rel  yy -> Customer\n",
            "}\n",
        ),
        // A notion with only a `def`: no padding group to compute.
        concat!(
            "notion Bare state {\n",
            "  def  \"Nothing but a definition.\"\n",
            "}\n",
        ),
        // Escapes inside the `def` string.
        concat!(
            "notion Quoted actor {\n",
            "  def  \"She said \\\"yes\\\", with a \\\\ in it.\"\n",
            "}\n",
        ),
    ];
    for src in notions {
        let parsed = parse_notion_file(&RepoPath::new("telos/notions/X.tel"), src)
            .unwrap_or_else(|d| panic!("must parse:\n{src}\n({} diagnostics)", d.len()));
        assert_eq!(emit_file(&TelFile::Notion(parsed)), src);
    }

    let constraints = [
        // A machine-checkable rule and a scoped constraint, with no `check`.
        concat!(
            "constraint CON-0009 quality \"Balances stay positive\" {\n",
            "  rule  Invoice.balance >= 0 and not Invoice.state == cancelled\n",
            "  scope INT-0017, INT-0042\n",
            "}\n",
        ),
        concat!(
            "constraint CON-0010 security \"No secrets in the repo\" {\n",
            "  rule  \"Secrets live in the vault.\"\n",
            "  scope global\n",
            "  check \"scripts/scan-secrets.sh\"\n",
            "}\n",
        ),
    ];
    for src in constraints {
        let parsed = parse_constraint_file(&RepoPath::new("telos/constraints/X.tel"), src)
            .unwrap_or_else(|d| panic!("must parse:\n{src}\n({} diagnostics)", d.len()));
        assert_eq!(emit_file(&TelFile::Constraint(parsed)), src);
    }

    let intents = [
        // Several `given` steps, several `then` steps, two scenarios, and
        // the full relation triple.
        concat!(
            "intent INT-12345 \"Five-digit ids are not padded away\" {\n",
            "  status deprecated\n",
            "  telos  \"Ids grow past four digits.\"\n",
            "  statement unwanted {\n",
            "    if     Invoice.balance < 0 or Invoice.state in (cancelled, open)\n",
            "    system shall set Invoice.state = cancelled\n",
            "  }\n",
            "  refines INT-0001\n",
            "  requires INT-0002\n",
            "  excludes INT-0003\n",
            "\n",
            "  scenario SCN-0001 \"first\" {\n",
            "    given Invoice { state: open, balance: \"120.00 EUR\" }\n",
            "    given Customer {}\n",
            "    when  PaymentReceived { amount: \"120.00 EUR\", at: 2026-01-31T12:00:00Z }\n",
            "    then  Invoice.state == settled\n",
            "    then  not Invoice.balance > 0\n",
            "  }\n",
            "\n",
            "  scenario SCN-0002 \"second\" {\n",
            "    given Invoice { due: 2026-01-31, late: true, count: -3, rate: 1.50 }\n",
            "    when  PaymentReceived {}\n",
            "    then  Invoice.state == settled\n",
            "  }\n",
            "}\n",
        ),
        // A draft intent whose statement is a free clause.
        concat!(
            "intent INT-0100 \"Prose is allowed in the shall clause\" {\n",
            "  status draft\n",
            "  telos  \"Some obligations resist formalization.\"\n",
            "  statement optional {\n",
            "    where  late-fees\n",
            "    system shall \"charge a \\\"late\\\" fee\"\n",
            "  }\n",
            "}\n",
        ),
    ];
    for src in intents {
        let parsed = parse_intent_file(&RepoPath::new("telos/intents/X.tel"), src)
            .unwrap_or_else(|d| panic!("must parse:\n{src}\n({} diagnostics)", d.len()));
        assert_eq!(emit_file(&TelFile::Intent(parsed)), src);
    }
}
