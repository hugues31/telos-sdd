//! End-to-end tests for `telos change open|list|abandon`, the `changing`
//! state reported by `status`, `show CHG-…`, refusal of `change open` on
//! unclaimed drift, and refusal of `check --sealed` while a change is open.
//!
//! Every test drives the real binary against the sealed `billing` corpus
//! fixture. What they prove is the CLI contract -- the frozen result shapes
//! in the result schema, the exact bytes of what lands on disk, and the
//! never-reuse rule for identifiers -- not the engine, which has its own tests
//! in `telos-core`.

mod common;

use std::fs;

use serde_json::{Value, json};

use common::{canonical_payload, telos, with_fixture};

/// Parses a command's stdout as a JSON envelope.
fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// The motivation every test uses when opening a change.
const MOTIVATION: &str = "Invoices can be settled";

/// The first change's file, relative to the repository root.
const CHG_0001: &str = "telos/changes/CHG-0001.tel";

/// The bytes `telos change open` must leave at [`CHG_0001`]: a change with
/// no op yet, in the canonical form `emit_change` defines.
const CANONICAL_OPEN_CHANGE: &str =
    "change CHG-0001 \"Invoices can be settled\" {\n  status open\n}\n";

/// The `telos/contexts/billing/notions/Invoice.tel` path, drifted by several tests.
const INVOICE_TEL: &str = "telos/contexts/billing/notions/Invoice.tel";

/// The exact `TELOS_DRIFT_DETECTED` hint, frozen by `docs/contracts.md`.
const DRIFT_HINT: &str = "run `telos status` to see drifted paths; capture with `telos adopt` or restore with `telos revert`";

/// The obligations of an `open` change, frozen by the result schema.
fn open_obligations() -> Value {
    json!(["stage the delta", "approve", "reconcile"])
}

/// Runs `telos change open <motivation>` and returns the allocated id.
fn open_change(dir: &std::path::Path, motivation: &str) -> String {
    let out = telos(dir, &["change", "open", motivation, "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "`telos change open`: expected exit 0, got {:?} -- {}",
        out.status,
        String::from_utf8_lossy(&out.stdout)
    );
    json_stdout(&out)["result"]["id"]
        .as_str()
        .expect("`change open` reports the allocated id")
        .to_string()
}

/// Appends one byte to a sealed spec file: the minimal unclaimed drift.
fn drift(dir: &std::path::Path) {
    let path = dir.join(INVOICE_TEL);
    let mut content = fs::read_to_string(&path).unwrap();
    content.push('\n');
    fs::write(&path, content).unwrap();
}

// --- change open ---------------------------------------------------------

/// The whole envelope, on a freshly sealed fixture, equals the result schema's
/// golden: the result pair and the single next action, byte for byte as a
/// `Value`.
#[test]
fn change_open_json_matches_the_golden_envelope() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["change", "open", MOTIVATION, "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(
        json_stdout(&out),
        json!({
            "ok": true,
            "command": "change",
            "result": { "id": "CHG-0001", "status": "open" },
            "error": null,
            "next_actions": ["telos add intent --change CHG-0001"]
        })
    );
}

/// The file `change open` writes is canonical to the byte -- it is
/// `emit_change` that wrote it, and byte-exact round-tripping is what makes it
/// readable back.
#[test]
fn change_open_writes_the_canonical_change_file_byte_for_byte() {
    let tmp = with_fixture();

    open_change(tmp.path(), MOTIVATION);

    assert_eq!(
        fs::read_to_string(tmp.path().join(CHG_0001)).unwrap(),
        CANONICAL_OPEN_CHANGE
    );
}

/// `counters.toml` is persisted at the corpus' own floors, with `change`
/// alone advanced by the allocation: the billing corpus tops out at
/// INT-0042, SCN-0107 and CON-0003, none of which this command allocated.
#[test]
fn change_open_persists_the_counters_at_the_corpus_floors() {
    let tmp = with_fixture();

    open_change(tmp.path(), MOTIVATION);

    assert_eq!(
        fs::read_to_string(tmp.path().join("telos/changes/counters.toml")).unwrap(),
        "intent = 42\nscenario = 107\nconstraint = 3\nchange = 1\n"
    );
}

#[test]
fn a_second_change_open_allocates_the_next_id() {
    let tmp = with_fixture();

    assert_eq!(open_change(tmp.path(), MOTIVATION), "CHG-0001");
    assert_eq!(open_change(tmp.path(), "another motivation"), "CHG-0002");
}

/// The floor scan preserves allocation when `counters.toml` is not to be
/// trusted -- a bad merge resolution, or the file simply gone -- *and* when
/// the change on disk cannot be parsed to be scanned for its ops: its id is
/// still read off its file name, so the next allocation lands past it
/// instead of reissuing it over a file that already exists.
#[test]
fn an_unparseable_change_still_holds_the_counter_down_without_counters_toml() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    fs::write(tmp.path().join(CHG_0001), "@@@ not a change @@@\n").unwrap();
    fs::remove_file(tmp.path().join("telos/changes/counters.toml")).unwrap();

    assert_eq!(open_change(tmp.path(), "after the damage"), "CHG-0002");
    assert_eq!(
        fs::read_to_string(tmp.path().join(CHG_0001)).unwrap(),
        "@@@ not a change @@@\n",
        "the damaged file must not have been overwritten by a reissued id"
    );
}

// --- status: the changing state ------------------------------------------

/// With one change open and nothing drifted, `status` reports `changing`,
/// the real `changes[]` result and the one relevant next action.
#[test]
fn status_json_with_an_open_change_reports_the_changing_state() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["status", "--json"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(
        json_stdout(&out),
        json!({
            "ok": true,
            "command": "status",
            "result": {
                "state": "changing",
                "changes": [{
                    "id": "CHG-0001",
                    "status": "open",
                    "obligations": ["stage the delta", "approve", "reconcile"]
                }],
                "drift": null,
                "coverage": {
                    "notions": 4,
                    "constraints": 1,
                    "intents_total": 2,
                    "intents_active": 2,
                    "scenarios_total": 2,
                    "scenarios_proved": 2,
                    "intents_implemented": 1
                }
            },
            "error": null,
            "next_actions": ["telos change list"]
        })
    );
}

// --- change abandon ------------------------------------------------------

#[test]
fn change_abandon_reports_the_abandoned_id_and_deletes_the_file() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["change", "abandon", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["command"], json!("change"));
    assert_eq!(
        envelope["result"],
        json!({ "id": "CHG-0001", "status": "abandoned" })
    );
    assert!(
        !tmp.path().join(CHG_0001).exists(),
        "the change file must be gone"
    );

    // Nothing is left open -- in particular `counters.toml`, which stays
    // behind in `telos/changes/`, is not mistaken for a change file.
    let out = telos(tmp.path(), &["change", "list", "--json"])
        .output()
        .unwrap();
    assert_eq!(json_stdout(&out)["result"], json!({ "changes": [] }));
    let out = telos(tmp.path(), &["status", "--json"]).output().unwrap();
    assert_eq!(json_stdout(&out)["result"]["state"], json!("coherent"));
}

/// A malformed id is a domain error under the same code as an id that does
/// not exist, not a clap usage error: an agent reading the envelope finds
/// both under `TELOS_REFERENCE_UNKNOWN`.
#[test]
fn change_abandon_of_a_malformed_id_is_a_domain_error() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["change", "abandon", "nonsense", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(
        envelope["error"]["message"],
        json!("cannot parse `nonsense` as a change id")
    );
}

/// An abandoned id is never handed out again because the counters file
/// remembers it even after its change file is deleted.
#[test]
fn an_abandoned_id_is_never_reused() {
    let tmp = with_fixture();
    assert_eq!(open_change(tmp.path(), MOTIVATION), "CHG-0001");
    assert_eq!(open_change(tmp.path(), "second"), "CHG-0002");

    telos(tmp.path(), &["change", "abandon", "CHG-0001"])
        .assert()
        .success();

    assert_eq!(open_change(tmp.path(), "third"), "CHG-0003");
}

/// Abandoning an id the store does not hold is the change store's own
/// `TELOS_REFERENCE_UNKNOWN`, with its nearest-id hint.
#[test]
fn change_abandon_of_an_unknown_id_reports_the_store_error() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["change", "abandon", "CHG-9999", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(
        envelope["error"]["message"],
        json!("unknown change `CHG-9999`")
    );
    assert_eq!(envelope["error"]["hint"], json!("closest is CHG-0001"));
}

// --- change list ---------------------------------------------------------

#[test]
fn change_list_json_reports_id_status_motivation_and_obligations() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    open_change(tmp.path(), "second motivation");

    let out = telos(tmp.path(), &["change", "list", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["command"], json!("change"));
    assert_eq!(
        envelope["result"],
        json!({
            "changes": [
                {
                    "id": "CHG-0001",
                    "status": "open",
                    "motivation": MOTIVATION,
                    "obligations": open_obligations()
                },
                {
                    "id": "CHG-0002",
                    "status": "open",
                    "motivation": "second motivation",
                    "obligations": open_obligations()
                }
            ]
        })
    );
}

#[test]
fn change_list_on_a_project_with_no_change_is_empty() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["change", "list", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(json_stdout(&out)["result"], json!({ "changes": [] }));
}

/// Human mode: one line per change, naming the id, the status and the
/// motivation.
#[test]
fn change_list_human_mode_prints_one_line_per_change() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["change", "list"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.lines().count(), 1, "one line per change: {stdout}");
    let line = stdout.lines().next().unwrap();
    assert!(line.contains("CHG-0001"), "line: {line}");
    assert!(line.contains("open"), "line: {line}");
    assert!(line.contains(MOTIVATION), "line: {line}");
}

/// An unparseable change file does not take `change list` down with it: the
/// entry is still reported with an empty motivation (there is
/// nothing trustworthy to read) and the repair obligation.
#[test]
fn change_list_is_best_effort_on_an_unparseable_change_file() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    fs::write(tmp.path().join(CHG_0001), "@@@ not a change @@@\n").unwrap();

    let out = telos(tmp.path(), &["change", "list", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(
        json_stdout(&out)["result"],
        json!({
            "changes": [{
                "id": "CHG-0001",
                "status": "open",
                "motivation": "",
                "obligations": ["repair telos/changes/CHG-0001.tel (unparseable)"]
            }]
        })
    );
}

// --- show CHG-… ----------------------------------------------------------

#[test]
fn show_of_a_change_matches_the_public_result_shape() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["show", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["command"], json!("show"));
    assert_eq!(
        envelope["result"],
        json!({
            "entity": {
                "id": "CHG-0001",
                "status": "open",
                "motivation": MOTIVATION,
                "ops": []
            },
            "canonical": CANONICAL_OPEN_CHANGE,
            "relations": { "out": [], "in": [] }
        })
    );
    // The same bytes that are on disk -- for a file telos wrote, the
    // canonical form and the file's text are one and the same.
    assert_eq!(
        envelope["result"]["canonical"],
        json!(fs::read_to_string(tmp.path().join(CHG_0001)).unwrap())
    );
}

/// `canonical` is the *file's* text, not a re-emission of the parsed
/// change: `telos/changes/` is outside the seal, so a hand-edited but
/// still parseable change file is legal state, and `show` must report what
/// is really on disk.
///
/// The discriminator is a valid file laid out non-canonically -- four
/// spaces of indentation where the emitter writes two. It parses (the
/// grammar is not whitespace-sensitive), so `entity` is fully reported,
/// while `emit_change` would render it back with two spaces: a `show` that
/// re-emitted would silently answer with bytes that are on nobody's disk.
#[test]
fn show_of_a_change_reports_the_file_text_not_a_re_emission() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let on_disk = "change CHG-0001 \"Invoices can be settled\" {\n    status open\n}\n";
    assert_ne!(
        on_disk, CANONICAL_OPEN_CHANGE,
        "the point of this test is that the two differ"
    );
    fs::write(tmp.path().join(CHG_0001), on_disk).unwrap();

    let out = telos(tmp.path(), &["show", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "a non-canonical but valid change file still shows: {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["result"]["canonical"], json!(on_disk));
    // The entity is still read through the parser, so it is unaffected by
    // the layout.
    assert_eq!(
        envelope["result"]["entity"],
        json!({
            "id": "CHG-0001",
            "status": "open",
            "motivation": MOTIVATION,
            "ops": []
        })
    );

    // Human mode prints the same file text.
    let out = telos(tmp.path(), &["show", "CHG-0001"]).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{on_disk}\nrelations:\n")
    );
}

#[test]
fn show_of_an_unknown_change_reports_the_store_error() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["show", "CHG-9999", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(
        envelope["error"]["message"],
        json!("unknown change `CHG-9999`")
    );
    assert_eq!(envelope["error"]["hint"], json!("closest is CHG-0001"));
}

/// Human mode prints the change's canonical text, then the (always empty)
/// relations section a change has -- a change is not a graph node.
#[test]
fn show_of_a_change_human_mode_prints_the_canonical_text_and_no_relation() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["show", "CHG-0001"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{CANONICAL_OPEN_CHANGE}\nrelations:\n")
    );
}

// --- drift gate --------------------------------------------------------------

#[test]
fn change_open_on_a_drifted_project_is_refused() {
    let tmp = with_fixture();
    drift(tmp.path());

    let out = telos(tmp.path(), &["change", "open", MOTIVATION, "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_DRIFT_DETECTED"));
    assert_eq!(envelope["error"]["hint"], json!(DRIFT_HINT));
    assert!(
        !tmp.path().join(CHG_0001).exists(),
        "a refused `change open` allocates nothing"
    );
}

/// `list` and `abandon` remain available while drifted: they are the two ways
/// out of a mess, not more mutation of the spec.
#[test]
fn change_list_and_abandon_are_allowed_while_drifted() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    drift(tmp.path());

    telos(tmp.path(), &["change", "list"]).assert().success();
    telos(tmp.path(), &["change", "abandon", "CHG-0001"])
        .assert()
        .success();
    assert!(!tmp.path().join(CHG_0001).exists());
}

// --- check --sealed while changing -----------------------------------------

#[test]
fn check_sealed_while_a_change_is_open_reports_change_state_invalid() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["check", "--sealed", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(
        envelope["error"]["code"],
        json!("TELOS_CHANGE_STATE_INVALID")
    );
    assert_eq!(
        envelope["error"]["message"],
        json!("open changes; reconcile or abandon them")
    );
    assert_eq!(envelope["error"]["hint"], json!("run `telos change list`"));
}

/// Drift still wins over an open change: state priority puts unclaimed drift
/// first, and `check --sealed` reports what it finds.
#[test]
fn check_sealed_reports_drift_before_open_changes() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    drift(tmp.path());

    let out = telos(tmp.path(), &["check", "--sealed", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    assert_eq!(
        json_stdout(&out)["error"]["code"],
        json!("TELOS_DRIFT_DETECTED")
    );
}

/// `check` without `--sealed` never looks at state at all: an open change
/// changes nothing about whether the spec parses and holds together.
#[test]
fn check_without_sealed_passes_while_a_change_is_open() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    telos(tmp.path(), &["check"]).assert().success();
}

// --- change diff / change approve -----------------------------------------

/// Runs one staging command (`add`/`edit`) with `payload` on stdin and
/// returns its `result`, asserting only that it succeeded -- the same
/// helper `tests/mutate.rs` uses, duplicated because integration test
/// binaries share no code but `common`.
fn stage_ok(dir: &std::path::Path, args: &[&str], payload: &str) -> Value {
    let mut cmd = telos(dir, args);
    let out = cmd
        .write_stdin(canonical_payload(args, payload))
        .output()
        .unwrap();
    let envelope = json_stdout(&out);
    assert_eq!(
        envelope["ok"],
        json!(true),
        "expected success, got {envelope}"
    );
    envelope["result"].clone()
}

/// A small `add notion` payload with no attrs or rels -- the minimal 1-op
/// change `change diff`'s golden test stages.
fn vendor_payload() -> String {
    json!({"owner": "billing", "name": "Vendor", "kind": "actor", "def": "A party the business pays."}).to_string()
}

/// The canonical block `add notion` with [`vendor_payload`] produces --
/// `emit_notion`'s own output, unindented, the exact bytes `change diff`
/// must report as the op's `after`.
const VENDOR_CANONICAL: &str =
    "notion billing/Vendor actor {\n  def  \"A party the business pays.\"\n}\n";

/// Whether `digest` is `sha256:` followed by exactly 64 lowercase hex
/// digits -- the shape `change diff`/`change approve` must report,
/// without pinning the value itself.
fn is_sha256_hex(digest: &str) -> bool {
    match digest.strip_prefix("sha256:") {
        Some(hex) => {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }
        None => false,
    }
}

#[test]
fn change_diff_on_a_one_op_add_reports_null_before_and_the_canonical_after() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &vendor_payload(),
    );

    let out = telos(tmp.path(), &["change", "diff", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["command"], json!("change"));
    let result = &envelope["result"];
    assert_eq!(result["id"], json!("CHG-0001"));
    assert_eq!(result["status"], json!("drafted"));
    assert_eq!(result["approved_digest"], Value::Null);
    assert_eq!(result["stale"], json!(false));
    let digest = result["digest"].as_str().expect("digest is a string");
    assert!(is_sha256_hex(digest), "not sha256:<64 hex>: {digest}");
    assert_eq!(
        result["ops"],
        json!([{
            "n": 1,
            "op": "add",
            "entity": "notion",
            "key": "billing/Vendor",
            "before": null,
            "after": VENDOR_CANONICAL,
        }])
    );
    assert_eq!(
        envelope["next_actions"],
        json!([format!(
            "telos change approve CHG-0001 --expected-digest {digest}"
        )])
    );
}

/// `before` for an `edit` is the sealed base's own canonical block -- the
/// corpus' `INT-0017.tel` bytes, read straight off disk; byte-exact round-tripping
/// being exactly what makes that legitimate -- and `after` is the patched
/// intent's canonical block. Staging never touches the sealed file itself.
#[test]
fn change_diff_of_an_edit_reports_the_corpus_block_as_before_and_the_patch_as_after() {
    let tmp = with_fixture();
    let int_0017_path = tmp
        .path()
        .join("telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel");
    let before = fs::read_to_string(&int_0017_path).unwrap();
    open_change(tmp.path(), MOTIVATION);
    stage_ok(
        tmp.path(),
        &[
            "edit", "intent", "INT-0017", "--change", "CHG-0001", "--json",
        ],
        &json!({"telos": "An invoice must start its life open and unpaid -- reworded."})
            .to_string(),
    );

    let out = telos(tmp.path(), &["change", "diff", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    let op = &envelope["result"]["ops"][0];
    assert_eq!(op["n"], json!(1));
    assert_eq!(op["op"], json!("edit"));
    assert_eq!(op["entity"], json!("intent"));
    assert_eq!(op["key"], json!("INT-0017"));
    assert_eq!(op["before"], json!(before));

    // Staging wrote only the change file -- the sealed intent is untouched.
    assert_eq!(fs::read_to_string(&int_0017_path).unwrap(), before);

    let after = before.replace(
        "An invoice must start its life open and unpaid.",
        "An invoice must start its life open and unpaid -- reworded.",
    );
    assert_ne!(after, before, "the replacement must actually have applied");
    assert_eq!(op["after"], json!(after));
}

/// Human mode: a summary line, then one `#N verb entity key` section per
/// op with `before:`/`after:` blocks. Exact wording is free -- there is no
/// golden test for it, only for `--json` (the same policy `status`/`check`
/// document for their own human output).
#[test]
fn change_diff_human_mode_prints_per_op_sections() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &vendor_payload(),
    );

    let out = telos(tmp.path(), &["change", "diff", "CHG-0001"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("#1 add notion billing/Vendor"), "{stdout}");
    assert!(stdout.contains("before: (none)"), "{stdout}");
    assert!(
        stdout.contains("after:\nnotion billing/Vendor actor {"),
        "{stdout}"
    );
}

/// `approve` writes `status approved` and a `digest` line into the change
/// file in `emit_change`'s layout, byte for byte. This test reads the exact
/// digest from the JSON result; the core digest tests separately pin the
/// algorithm to a golden value.
#[test]
fn change_approve_writes_status_and_digest_and_matches_the_golden_result() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &vendor_payload(),
    );

    let out = telos(tmp.path(), &["change", "approve", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["command"], json!("change"));
    assert_eq!(envelope["result"]["id"], json!("CHG-0001"));
    assert_eq!(envelope["result"]["status"], json!("approved"));
    let digest = envelope["result"]["digest"]
        .as_str()
        .expect("digest is a string")
        .to_string();
    assert!(is_sha256_hex(&digest), "not sha256:<64 hex>: {digest}");
    assert_eq!(
        envelope["next_actions"],
        json!(["telos change reconcile CHG-0001"])
    );

    let expected = format!(
        "change CHG-0001 \"{MOTIVATION}\" {{\n  \
           status approved\n  \
           digest \"{digest}\"\n\
         \n  \
           op add notion billing/Vendor actor {{\n    \
             def  \"A party the business pays.\"\n  \
           }}\n\
         }}\n"
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join(CHG_0001)).unwrap(),
        expected
    );
}

#[test]
fn change_approve_refuses_a_digest_that_changed_during_review() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &vendor_payload(),
    );
    let diff = json_stdout(
        &telos(tmp.path(), &["change", "diff", "CHG-0001", "--json"])
            .output()
            .unwrap(),
    );
    let stale = diff["result"]["digest"].as_str().unwrap().to_string();
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &vendor_payload().replace("Vendor", "Supplier"),
    );
    let before = fs::read_to_string(tmp.path().join(CHG_0001)).unwrap();

    let out = telos(
        tmp.path(),
        &[
            "change",
            "approve",
            "CHG-0001",
            "--expected-digest",
            &stale,
            "--json",
        ],
    )
    .output()
    .unwrap();
    let envelope = json_stdout(&out);

    assert!(!out.status.success(), "{envelope}");
    assert_eq!(
        envelope["error"]["code"],
        json!("TELOS_CHANGE_STATE_INVALID")
    );
    assert_eq!(
        envelope["error"]["message"],
        json!("change CHG-0001 no longer matches the expected digest")
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join(CHG_0001)).unwrap(),
        before
    );
    assert!(!before.contains("status approved"));
    assert!(!before.contains("\n  digest \""));
}

/// `approve` of a change with no staged op is refused, with the exact
/// documented message and hint -- and writes nothing: the file is
/// left exactly as `change open` wrote it.
#[test]
fn change_approve_of_a_change_with_no_ops_is_change_state_invalid() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);

    let out = telos(tmp.path(), &["change", "approve", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(
        envelope["error"]["code"],
        json!("TELOS_CHANGE_STATE_INVALID")
    );
    assert_eq!(
        envelope["error"]["message"],
        json!("change CHG-0001 has no staged operations")
    );
    assert_eq!(
        envelope["error"]["hint"],
        json!("stage operations with telos add|edit|remove first")
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join(CHG_0001)).unwrap(),
        CANONICAL_OPEN_CHANGE
    );
}

/// Staging into an already-approved change is allowed (`mutate.rs`'s own
/// design): nothing is lost, but the approval goes stale -- `diff`
/// reports it, and re-`approve` idempotently clears it by recalculating the
/// digest.
#[test]
fn staging_after_approve_goes_stale_and_re_approve_clears_it() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &vendor_payload(),
    );

    let approve_out = telos(tmp.path(), &["change", "approve", "CHG-0001", "--json"])
        .output()
        .unwrap();
    assert!(approve_out.status.success());
    let first_digest = json_stdout(&approve_out)["result"]["digest"]
        .as_str()
        .unwrap()
        .to_string();

    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &json!({
            "name": "InvoiceCancelled", "kind": "event",
            "def": "An invoice was cancelled before settlement."
        })
        .to_string(),
    );

    let diff_out = telos(tmp.path(), &["change", "diff", "CHG-0001", "--json"])
        .output()
        .unwrap();
    let envelope = json_stdout(&diff_out);
    let result = &envelope["result"];
    assert_eq!(result["status"], json!("approved"));
    assert_eq!(result["stale"], json!(true));
    assert_eq!(result["approved_digest"], json!(first_digest));
    let live_digest = result["digest"].as_str().unwrap().to_string();
    assert_ne!(live_digest, first_digest);
    assert_eq!(
        envelope["next_actions"],
        json!([format!(
            "telos change approve CHG-0001 --expected-digest {live_digest}"
        )])
    );

    let reapprove_out = telos(tmp.path(), &["change", "approve", "CHG-0001", "--json"])
        .output()
        .unwrap();
    assert!(reapprove_out.status.success());
    let second_digest = json_stdout(&reapprove_out)["result"]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(second_digest, live_digest);
    assert_ne!(second_digest, first_digest);

    let diff_out2 = telos(tmp.path(), &["change", "diff", "CHG-0001", "--json"])
        .output()
        .unwrap();
    let envelope2 = json_stdout(&diff_out2);
    assert_eq!(envelope2["result"]["stale"], json!(false));
    assert_eq!(envelope2["result"]["approved_digest"], json!(second_digest));
    assert_eq!(
        envelope2["next_actions"],
        json!(["telos change reconcile CHG-0001"])
    );
}

/// `approve` is refused on unclaimed drift, same as `open`, and writes
/// nothing -- the change stays `drafted`, no digest.
#[test]
fn change_approve_on_unclaimed_drift_is_refused() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &vendor_payload(),
    );
    drift(tmp.path());

    let out = telos(tmp.path(), &["change", "approve", "CHG-0001", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_DRIFT_DETECTED"));
    assert_eq!(envelope["error"]["hint"], json!(DRIFT_HINT));

    let contents = fs::read_to_string(tmp.path().join(CHG_0001)).unwrap();
    assert!(contents.contains("status drafted"), "{contents}");
    assert!(!contents.contains("digest"), "{contents}");
}

/// Unlike `approve`, `diff` is allowed on unclaimed drift -- it reads,
/// it does not stage a review against the base.
#[test]
fn change_diff_is_allowed_on_unclaimed_drift() {
    let tmp = with_fixture();
    open_change(tmp.path(), MOTIVATION);
    stage_ok(
        tmp.path(),
        &["add", "notion", "--change", "CHG-0001", "--json"],
        &vendor_payload(),
    );
    drift(tmp.path());

    telos(tmp.path(), &["change", "diff", "CHG-0001"])
        .assert()
        .success();
}
