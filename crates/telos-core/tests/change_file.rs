//! Change-file suite: the public `parse_change_file` seen from outside the
//! crate.
//!
//! Two angles:
//!
//! 1. **Byte-exact round-trip** -- for every canonical change source below,
//!    `emit_change(&parse_change_file(p, s)) == s`. The emitter is the
//!    definition of the canonical form, so this is the only statement
//!    that keeps the parser and the emitter from drifting apart: padding,
//!    the blank line before each op, the indentation of nested entity
//!    blocks and the staged order of the ops are all pinned at once. The
//!    the canonical change and journal examples are round-tripped in
//!    `syntax/parser.rs`'s unit tests, where
//!    the single in-crate copy of each golden lives (next to the model
//!    fixture it is paired with); the sources here cover the shapes they do
//!    not.
//! 2. **Diagnostics** -- the parse-level rules a change file has beyond its
//!    grammar (the digest belongs to an approved change and to no other and
//!    it is a `sha256:<64 hex>`; a journal belongs to an implementing change
//!    and to no other), plus the arity and vocabulary errors an agent is
//!    most likely to write.

use telos_core::emit::emit_change;
use telos_core::error::{Diagnostic, ErrorCode};
use telos_core::ids::{ChangeId, RepoPath};
use telos_core::model::{Binding, ChangeStatus, JournalEntry, StagedOp};
use telos_core::syntax::parse_change_file;

fn path() -> RepoPath {
    RepoPath::new("telos/changes/CHG-0001.tel")
}

/// Parses a source that must be valid, reporting every diagnostic when it
/// is not.
fn parse_ok(src: &str) -> telos_core::model::Change {
    parse_change_file(&path(), src).unwrap_or_else(|diags| {
        let messages: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
        panic!("must parse:\n{src}\ngot: {messages:#?}")
    })
}

/// Parses a source that must fail, returning its diagnostics.
fn parse_err(src: &str) -> Vec<Diagnostic> {
    match parse_change_file(&path(), src) {
        Ok(change) => panic!("must not parse, got {change:#?}"),
        Err(diags) => diags,
    }
}

/// Asserts that some diagnostic carries `needle`, and that they are all
/// parse errors tagged with the file they came from.
fn assert_reports(diags: &[Diagnostic], needle: &str) {
    for diag in diags {
        assert_eq!(diag.code, ErrorCode::TelosParseError);
        assert_eq!(diag.file.as_ref(), Some(&path()));
        assert!(diag.line.is_some() && diag.col.is_some(), "{diag:?}");
    }
    let found: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
    assert!(
        found.iter().any(|m| m.contains(needle)),
        "expected a diagnostic containing {needle:?}, got {found:#?}"
    );
}

// --- the canonical variants ----------------------------------------------

/// `telos change open` and nothing else: no op, no digest.
const OPEN_EMPTY: &str = concat!(
    "change CHG-0001 \"Nothing is staged yet\" {\n",
    "  status open\n",
    "}\n",
);

/// One `add` op, so one nested `notion-file` block.
const DRAFTED_ONE_ADD: &str = concat!(
    "change CHG-0002 \"Introduce the ledger\" {\n",
    "  status drafted\n",
    "\n",
    "  op add notion billing/Ledger entity {\n",
    "    def    \"A record of every posting.\"\n    phrase \"ledger\"\n",
    "    attr   balance money\n",
    "  }\n",
    "}\n",
);

/// An approved change with one op of every entity kind, each nested block a
/// different entity file rule -- and the intent one carrying a statement block
/// and a scenario, so the deepest nesting the grammar admits is exercised.
const APPROVED_MULTI: &str = concat!(
    "change CHG-0003 \"Rework the settlement rules\" {\n",
    "  status approved\n",
    "  digest \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
    "\n",
    "  op edit notion billing/Invoice entity {\n",
    "    def    \"A bill issued to a Customer.\"\n    phrase \"invoice\"\n",
    "    attr   state enum(open, settled)\n",
    "    rel    issued-to -> Customer\n",
    "  }\n",
    "\n",
    "  op add intent INT-0042 in billing/settlement \"Payment settles the invoice\" {\n",
    "    status active\n",
    "    telos  \"Customers must see their debt cleared.\"\n",
    "    statement event-driven {\n",
    "      when   PaymentReceived on Invoice\n",
    "      system shall set Invoice.state = settled\n",
    "    }\n",
    "\n",
    "    scenario SCN-0107 \"full payment settles the invoice\" {\n",
    "      given Invoice { state: open }\n",
    "      when  PaymentReceived {}\n",
    "      then  Invoice.state == settled\n",
    "    }\n",
    "  }\n",
    "\n",
    "  op add constraint CON-0009 in capability billing/settlement quality \"Balances stay positive\" {\n",
    "    rule  Invoice.balance >= 0\n",
    "    check \"git --version\"\n",
    "  }\n",
    "}\n",
);

/// The ops with no entity block at all: three `remove`s and an `accept`.
const REMOVE_AND_ACCEPT: &str = concat!(
    "change CHG-0004 \"Retire the legacy pieces\" {\n",
    "  status drafted\n",
    "\n",
    "  op remove notion billing/Ledger\n",
    "\n",
    "  op remove intent INT-0017 from billing/invoicing\n",
    "\n",
    "  op remove constraint CON-0003 from billing\n",
    "\n",
    "  op accept \"telos/telos.toml\" \"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\"\n",
    "}\n",
);

/// The status an approved change moves on to during implementation.
/// It keeps the digest
/// its approval froze -- reconcile accepts `approved` *or* `implementing`,
/// and a change that lost its digest on the way could never be reconciled.
const IMPLEMENTING_WITH_DIGEST: &str = concat!(
    "change CHG-0005 \"Settle the ledger\" {\n",
    "  status implementing\n",
    "  digest \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
    "\n",
    "  op remove notion billing/Ledger\n",
    "}\n",
);

/// An implementing change whose journal holds runs only: the state
/// `telos test` leaves behind before anything is bound.
const IMPLEMENTING_RUNS_ONLY: &str = concat!(
    "change CHG-0006 \"Settle the ledger\" {\n",
    "  status implementing\n",
    "  digest \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
    "\n",
    "  op remove notion billing/Ledger\n",
    "\n",
    "  run  SCN-0107 red \"tests/billing.rs::scn_0107_full_payment\" \"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\"\n",
    "  run  SCN-0107 green \"tests/billing.rs::scn_0107_full_payment\" \"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\"\n",
    "}\n",
);

/// Runs *and* binds, in the order they were appended -- a bind between two
/// runs, which no sort would ever produce.
const IMPLEMENTING_RUNS_AND_BINDS: &str = concat!(
    "change CHG-0007 \"Settle the ledger\" {\n",
    "  status implementing\n",
    "  digest \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
    "\n",
    "  op remove notion billing/Ledger\n",
    "\n",
    "  run  SCN-0107 red \"tests/billing.rs::scn_0107_full_payment\" \"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\"\n",
    "  bind \"src/billing.rs\" -> INT-0042\n",
    "  run  SCN-0107 green \"tests/billing.rs\" \"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\"\n",
    "  bind \"src/ledger.rs\" -> INT-0042\n",
    "}\n",
);

fn variants() -> [&'static str; 7] {
    [
        OPEN_EMPTY,
        DRAFTED_ONE_ADD,
        APPROVED_MULTI,
        REMOVE_AND_ACCEPT,
        IMPLEMENTING_WITH_DIGEST,
        IMPLEMENTING_RUNS_ONLY,
        IMPLEMENTING_RUNS_AND_BINDS,
    ]
}

#[test]
fn every_canonical_variant_round_trips_byte_exact() {
    for src in variants() {
        assert_eq!(emit_change(&parse_ok(src)), src, "not byte-exact:\n{src}");
    }
}

#[test]
fn every_canonical_variant_survives_a_second_round_trip() {
    for src in variants() {
        let once = emit_change(&parse_ok(src));
        let twice = emit_change(&parse_ok(&once));
        assert_eq!(once, twice, "not stable on a second round-trip:\n{src}");
    }
}

#[test]
fn an_open_change_has_no_op_and_no_digest() {
    let change = parse_ok(OPEN_EMPTY);
    assert_eq!(change.id, ChangeId(1));
    assert_eq!(change.motivation, "Nothing is staged yet");
    assert_eq!(change.status, ChangeStatus::Open);
    assert_eq!(change.approved_digest, None);
    assert!(change.ops.is_empty());
}

#[test]
fn the_digest_field_is_the_only_source_of_the_approved_digest() {
    let change = parse_ok(APPROVED_MULTI);
    assert_eq!(change.status, ChangeStatus::Approved);
    assert_eq!(
        change.approved_digest.as_deref(),
        Some("sha256:0000000000000000000000000000000000000000000000000000000000000000")
    );
    // The parser reports what the file says, and never recomputes it: a
    // change whose delta moved since approval must stay detectable.
    assert!(change.is_stale());
    assert_eq!(parse_ok(DRAFTED_ONE_ADD).approved_digest, None);
}

#[test]
fn ops_keep_their_staged_order() {
    // Op order is data, never sorted -- these four are in reverse
    // id order on purpose.
    let ops = parse_ok(REMOVE_AND_ACCEPT).ops;
    let shape: Vec<(&str, &str, String)> = ops
        .iter()
        .map(|op| (op.verb(), op.entity(), op.key()))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("remove", "notion", "billing/Ledger".to_string()),
            ("remove", "intent", "INT-0017".to_string()),
            ("remove", "constraint", "CON-0003".to_string()),
            ("accept", "file", "telos/telos.toml".to_string()),
        ]
    );
    let StagedOp::Accept { path, oid } = &ops[3] else {
        panic!("the fourth op is an accept");
    };
    assert_eq!(path.as_str(), "telos/telos.toml");
    assert_eq!(oid.0, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
}

#[test]
fn add_and_edit_carry_the_whole_nested_entity() {
    let ops = parse_ok(APPROVED_MULTI).ops;
    assert_eq!(ops.len(), 3);
    let StagedOp::EditOwnedNotion { notion, .. } = &ops[0] else {
        panic!("the first op edits a notion");
    };
    assert_eq!(notion.name.as_str(), "Invoice");
    assert_eq!(notion.attrs.len(), 1);
    assert_eq!(notion.rels.len(), 1);
    let StagedOp::AddOwnedIntent { intent, .. } = &ops[1] else {
        panic!("the second op adds an intent");
    };
    assert_eq!(intent.title, "Payment settles the invoice");
    assert_eq!(intent.scenarios.len(), 1);
    let StagedOp::AddOwnedConstraint { constraint, .. } = &ops[2] else {
        panic!("the third op adds a constraint");
    };
    assert_eq!(constraint.check.as_deref(), Some("git --version"));
}

// --- diagnostics ----------------------------------------------------------

#[test]
fn an_unknown_status_names_the_five_it_could_have_been() {
    let diags = parse_err("change CHG-0001 \"x\" {\n  status finished\n}\n");
    assert_reports(
        &diags,
        "expected one of `open`, `drafted`, `approved`, `implementing`, `abandoned`",
    );
    assert_eq!(diags[0].line, Some(2));
}

#[test]
fn a_digest_on_a_change_that_carries_no_approval_is_rejected() {
    let src = concat!(
        "change CHG-0001 \"x\" {\n",
        "  status drafted\n",
        "  digest \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
        "}\n",
    );
    assert_reports(
        &parse_err(src),
        "digest is only valid on an approved or implementing change",
    );
}

#[test]
fn an_approved_or_implementing_change_without_a_digest_is_rejected() {
    for status in ["approved", "implementing"] {
        let src = format!("change CHG-0001 \"x\" {{\n  status {status}\n}}\n");
        assert_reports(&parse_err(&src), "an approved change must carry its digest");
    }
}

#[test]
fn an_implementing_change_keeps_the_digest_of_its_approval() {
    let change = parse_ok(IMPLEMENTING_WITH_DIGEST);
    assert_eq!(change.status, ChangeStatus::Implementing);
    assert_eq!(
        change.approved_digest.as_deref(),
        Some("sha256:0000000000000000000000000000000000000000000000000000000000000000")
    );
}

#[test]
fn a_digest_that_is_not_sha256_of_64_hex_is_rejected() {
    for digest in [
        "sha256:cafe",
        "cafe",
        "sha1:0000000000000000000000000000000000000000",
        // 64 characters, but not all of them hexadecimal.
        "sha256:z000000000000000000000000000000000000000000000000000000000000000",
        // Upper-case hex is not the canonical form.
        "sha256:9F8E7D6C5B4A39281706F5E4D3C2B1A09F8E7D6C5B4A39281706F5E4D3C2B1A0",
    ] {
        let src =
            format!("change CHG-0001 \"x\" {{\n  status approved\n  digest \"{digest}\"\n}}\n");
        assert_reports(
            &parse_err(&src),
            "malformed digest; expected sha256:<64 hex>",
        );
    }
}

#[test]
fn an_accept_op_with_a_path_but_no_oid_is_an_arity_error() {
    let src = concat!(
        "change CHG-0001 \"x\" {\n",
        "  status drafted\n",
        "\n",
        "  op accept \"telos/telos.toml\"\n",
        "}\n",
    );
    assert_reports(
        &parse_err(src),
        "expected the blob oid string after the path",
    );
}

#[test]
fn a_remove_op_naming_the_wrong_kind_of_id_says_which_one_it_wanted() {
    let cases = [
        ("op remove intent CON-0003", "expected an intent id"),
        ("op remove constraint INT-0017", "expected a constraint id"),
        (
            "op remove notion billing/INT-0017",
            "expected a capability id",
        ),
        (
            "op remove ledger Ledger",
            "expected `context`, `capability`, `notion`, `intent` or `constraint`",
        ),
    ];
    for (op, expected) in cases {
        let src = format!("change CHG-0001 \"x\" {{\n  status drafted\n\n  {op}\n}}\n");
        assert_reports(&parse_err(&src), expected);
    }
}

#[test]
fn an_unknown_verb_names_the_four_ops() {
    let src = concat!(
        "change CHG-0001 \"x\" {\n",
        "  status drafted\n",
        "\n",
        "  op delete notion Ledger\n",
        "}\n",
    );
    assert_reports(
        &parse_err(src),
        "expected `add`, `edit`, `remove`, `move` or `accept`",
    );
}

// --- the journal ------------------------------------------------

#[test]
fn a_change_with_no_journal_keeps_the_canonical_shape_exactly() {
    // The journal block is written only when there is one: every canonical variant
    // above round-trips unchanged, and none of them grew a blank line.
    for src in [
        OPEN_EMPTY,
        DRAFTED_ONE_ADD,
        APPROVED_MULTI,
        REMOVE_AND_ACCEPT,
        IMPLEMENTING_WITH_DIGEST,
    ] {
        let change = parse_ok(src);
        assert!(change.journal.is_empty(), "{src}");
        assert_eq!(emit_change(&change), src);
    }
}

#[test]
fn the_journal_is_read_back_in_append_order() {
    let change = parse_ok(IMPLEMENTING_RUNS_AND_BINDS);
    let shape: Vec<String> = change
        .journal
        .iter()
        .map(|entry| match entry {
            JournalEntry::Run(run) => {
                format!("run {} {}", run.scenario, run.witness.as_str())
            }
            JournalEntry::Bind { path, intent } => format!("bind {path} {intent}"),
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            "run SCN-0107 red",
            "bind src/billing.rs INT-0042",
            "run SCN-0107 green",
            "bind src/ledger.rs INT-0042",
        ]
    );
}

#[test]
fn a_journalled_change_claims_its_test_files_and_bound_paths() {
    // Drift on a file this change owns is legitimate work in progress.
    // in progress, admissible to *its* reconcile -- so it is claimed.
    let claims = parse_ok(IMPLEMENTING_RUNS_AND_BINDS).claims();
    for path in [
        "telos/contexts/billing/notions/Ledger.tel",
        "tests/billing.rs",
        "src/billing.rs",
        "src/ledger.rs",
    ] {
        assert!(claims.contains(&RepoPath::new(path)), "{path}: {claims:?}");
    }
    assert_eq!(claims.len(), 4);
}

/// The spec tree is written by ops and reconcile, never by journal entries.
/// Therefore no journal line may name a path under `telos/`, and a
/// change file that tries does not parse.
///
/// Asserting that some well-behaved fixture's `claims()` happens to exclude
/// `telos/contexts/billing/bindings.tel` would have proved nothing: a change file is a text
/// file, `claims()` returns what the journal holds, and reconcile's drift
/// gate admits any path the change claims. This is the test that closes
/// that door.
#[test]
fn a_journal_line_naming_the_spec_tree_does_not_parse() {
    for line in [
        "  bind \"telos/contexts/billing/bindings.tel\" -> INT-0042",
        "  run  SCN-0107 green \"telos/contexts/billing/bindings.tel\" \"e69de29\"",
        "  run  SCN-0107 red \"telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel::scn_0107\" \"e69de29\"",
    ] {
        let src = format!(
            "change CHG-0001 \"x\" {{\n  status implementing\n  digest \"sha256:{}\"\n\n\
             {line}\n}}\n",
            "0".repeat(64)
        );
        assert_reports(
            &parse_err(&src),
            "a journal line cannot name a path under telos/",
        );
    }
}

#[test]
fn the_journal_folds_into_bindings_greens_and_binds_only() {
    let bindings = parse_ok(IMPLEMENTING_RUNS_AND_BINDS).journal_bindings();
    let shape: Vec<String> = bindings
        .iter()
        .map(|binding| match binding {
            Binding::Implements { path, intent } => format!("implements {path} {}", intent.node),
            Binding::Proves { test, scenario } => format!("proves {test} {}", scenario.node),
        })
        .collect();
    // The red run contributes nothing; the order is the journal's.
    assert_eq!(
        shape,
        vec![
            "implements src/billing.rs INT-0042",
            "proves tests/billing.rs SCN-0107",
            "implements src/ledger.rs INT-0042",
        ]
    );
}

#[test]
fn a_journal_does_not_move_the_digest_of_the_delta_it_implements() {
    // The two sources below stage the same single op; only the journal
    // differs. An approval survives its own implementation.
    let bare = parse_ok(IMPLEMENTING_WITH_DIGEST);
    let journalled = parse_ok(IMPLEMENTING_RUNS_AND_BINDS);
    assert_eq!(bare.ops, journalled.ops);
    assert_eq!(bare.ops_digest(), journalled.ops_digest());
    assert!(!journalled.journal.is_empty());
}

#[test]
fn a_journal_on_a_change_that_is_not_implementing_is_rejected() {
    let src = concat!(
        "change CHG-0001 \"x\" {\n",
        "  status drafted\n",
        "\n",
        "  bind \"src/billing.rs\" -> INT-0042\n",
        "}\n",
    );
    assert_reports(
        &parse_err(src),
        "a journal is only valid on an implementing change",
    );
}

#[test]
fn a_verdict_that_is_neither_red_nor_green_is_rejected() {
    let src = concat!(
        "change CHG-0001 \"x\" {\n",
        "  status implementing\n",
        "  digest \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
        "\n",
        "  run  SCN-0107 blue \"tests/billing.rs\" \"cafe\"\n",
        "}\n",
    );
    assert_reports(&parse_err(src), "expected `red` or `green`, found `blue`");
}

#[test]
fn an_op_may_not_follow_a_journal_line() {
    let src = concat!(
        "change CHG-0001 \"x\" {\n",
        "  status implementing\n",
        "  digest \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
        "\n",
        "  bind \"src/billing.rs\" -> INT-0042\n",
        "\n",
        "  op remove notion billing/Ledger\n",
        "}\n",
    );
    assert_reports(&parse_err(src), "expected `run`, `bind` or `}`, found `op`");
}

#[test]
fn a_broken_nested_block_does_not_swallow_the_ops_that_follow() {
    // Error recovery is brace-depth aware: the bad `attr` line inside
    // the nested notion is skipped, the notion's `}` closes it, and the
    // `accept` op two lines down is still checked -- two diagnostics, not
    // one.
    let src = concat!(
        "change CHG-0001 \"x\" {\n",
        "  status drafted\n",
        "\n",
        "  op add notion billing/Ledger entity {\n",
        "    def    \"A record.\"\n    phrase \"ledger\"\n",
        "    attr   balance wobbly\n",
        "  }\n",
        "\n",
        "  op accept \"telos/telos.toml\"\n",
        "}\n",
    );
    let diags = parse_err(src);
    assert_reports(&diags, "expected an attribute type");
    assert_reports(&diags, "expected the blob oid string after the path");
    assert_eq!(diags.len(), 2, "{diags:#?}");
}

#[test]
fn a_change_rejects_legacy_entity_declarations_without_an_owner() {
    for declaration in [
        "notion Ledger entity {\n    def    \"A record.\"\n    phrase \"ledger\"\n  }",
        "intent INT-0042 \"Legacy intent\" {\n    status draft\n    telos  \"No owner.\"\n    statement ubiquitous {\n      system shall \"record it\"\n    }\n  }",
        "constraint CON-0009 quality \"Legacy constraint\" {\n    rule  \"No owner.\"\n    scope global\n  }",
    ] {
        let src = format!(
            "change CHG-0001 \"Legacy declaration\" {{\n  status drafted\n\n  op add {declaration}\n}}\n"
        );
        assert_reports(&parse_err(&src), "owner");
    }
}
