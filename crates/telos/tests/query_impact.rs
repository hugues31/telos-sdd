//! End-to-end tests for `telos query <type>` and `telos impact <id|Name>`:
//! per-type filters combined in AND, an inapplicable filter as a clap usage
//! error, and the reverse-closure `impact` report. Every test runs the real
//! binary against the sealed `billing` corpus fixture.

mod common;

use serde_json::{Value, json};

use common::{telos, with_fixture};

#[test]
fn query_requires_qualified_vocabulary_and_reports_domain_owners() {
    let tmp = with_fixture();
    let out = telos(
        tmp.path(),
        &[
            "query",
            "intent",
            "--context",
            "billing",
            "--capability",
            "settlement",
            "--using",
            "NOT:billing/Invoice",
            "--json",
        ],
    )
    .output()
    .unwrap();
    assert_eq!(
        json_stdout(&out)["result"]["items"],
        json!([{"id": "INT-0042", "owner": "billing/settlement"}])
    );

    let bare = telos(
        tmp.path(),
        &["query", "intent", "--using", "Invoice", "--json"],
    )
    .output()
    .unwrap();
    let error = json_stdout(&bare);
    assert_eq!(error["ok"], json!(false));
    assert_eq!(error["error"]["code"], json!("TELOS_PARSE_ERROR"));
}

/// Parses a command's stdout as a JSON envelope.
fn json_stdout(out: &std::process::Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn items(out: &std::process::Output) -> Vec<String> {
    json_stdout(out)["result"]["items"]
        .as_array()
        .expect("items is an array")
        .iter()
        .map(|value| {
            if let Some(item) = value.as_str() {
                return item.to_string();
            }
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                return id.to_string();
            }
            let name = &value["name"];
            if let Some(name) = name.as_str() {
                return name.to_string();
            }
            format!(
                "{}/{}",
                name["context"].as_str().expect("notion context"),
                name["notion"].as_str().expect("notion name")
            )
        })
        .collect()
}

// --- query intent: filters -------------------------------------------------

/// `--using <Notion>`: only intents whose statement (or its `on` clause, or
/// its `set` target) names the notion -- both INT-0017 and INT-0042 mention
/// `Invoice`.
#[test]
fn query_intent_using_invoice_matches_both_intents() {
    let tmp = with_fixture();

    let out = telos(
        tmp.path(),
        &[
            "query",
            "intent",
            "--using",
            "NOT:billing/Invoice",
            "--json",
        ],
    )
    .output()
    .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(items(&out), vec!["INT-0017", "INT-0042"]);
}

/// `--triggered-by <Event>`: only intents whose statement is event-driven by
/// this event -- only INT-0042 is driven by `PaymentReceived`.
#[test]
fn query_intent_triggered_by_payment_received_matches_only_int_0042() {
    let tmp = with_fixture();

    let out = telos(
        tmp.path(),
        &[
            "query",
            "intent",
            "--triggered-by",
            "NOT:billing/PaymentReceived",
            "--json",
        ],
    )
    .output()
    .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(items(&out), vec!["INT-0042"]);
}

/// All three intent filters combine in AND: active + uses Invoice + triggered
/// by InvoiceIssued narrows the corpus down to INT-0017 alone (INT-0042 is
/// triggered by `PaymentReceived`, not `InvoiceIssued`).
#[test]
fn query_intent_combines_status_using_and_triggered_by_in_and() {
    let tmp = with_fixture();

    let out = telos(
        tmp.path(),
        &[
            "query",
            "intent",
            "--status",
            "active",
            "--using",
            "NOT:billing/Invoice",
            "--triggered-by",
            "NOT:billing/InvoiceIssued",
            "--json",
        ],
    )
    .output()
    .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(items(&out), vec!["INT-0017"]);
}

/// No filters at all answers with every intent, natural-key sorted -- same
/// as `list intent`.
#[test]
fn query_intent_without_filters_lists_every_intent() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["query", "intent", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(items(&out), vec!["INT-0017", "INT-0042"]);
}

// --- query scenario / notion / constraint -----------------------------------

/// `query scenario --using Invoice`: both scenarios mention `Invoice` (one
/// in its `given`, the other in its `then`).
#[test]
fn query_scenario_using_invoice_matches_both_scenarios() {
    let tmp = with_fixture();

    let out = telos(
        tmp.path(),
        &[
            "query",
            "scenario",
            "--using",
            "NOT:billing/Invoice",
            "--json",
        ],
    )
    .output()
    .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(items(&out), vec!["SCN-0091", "SCN-0107"]);
}

/// `query notion --kind event`: only the two event notions, alphabetically.
#[test]
fn query_notion_kind_event_matches_the_two_events() {
    let tmp = with_fixture();

    let out = telos(
        tmp.path(),
        &["query", "notion", "--kind", "event", "--json"],
    )
    .output()
    .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(
        items(&out),
        vec!["billing/InvoiceIssued", "billing/PaymentReceived"]
    );
}

/// `query constraint --kind architecture`: the corpus's one constraint is
/// architecture-kind.
#[test]
fn query_constraint_kind_architecture_matches_con_0003() {
    let tmp = with_fixture();

    let out = telos(
        tmp.path(),
        &["query", "constraint", "--kind", "architecture", "--json"],
    )
    .output()
    .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert_eq!(items(&out), vec!["CON-0003"]);
}

// --- query: unknown notion argument -----------------------------------------

/// `--using Rogue`: no corpus notion is close enough to `Rogue` for a
/// suggestion (edit distance exceeds the threshold for all four), so the
/// hint stays `null` -- distinct from the typo case below.
#[test]
fn query_intent_using_an_unknown_notion_reports_reference_unknown_with_no_hint() {
    let tmp = with_fixture();

    let out = telos(
        tmp.path(),
        &["query", "intent", "--using", "NOT:billing/Rogue", "--json"],
    )
    .output()
    .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    let message = envelope["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("unknown notion `billing/Rogue`"),
        "message: {message}"
    );
    assert_eq!(envelope["error"]["hint"], Value::Null);
}

/// `--using Invoic` (a typo, one edit away from `Invoice`): close enough to
/// suggest -- the other suggestion path `--using Rogue` above does not take.
#[test]
fn query_intent_using_a_close_typo_reports_a_hint() {
    let tmp = with_fixture();

    let out = telos(
        tmp.path(),
        &["query", "intent", "--using", "NOT:billing/Invoic", "--json"],
    )
    .output()
    .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(
        envelope["error"]["hint"],
        json!("closest is `NOT:billing/Invoice`")
    );
}

// --- query: inapplicable filter is a usage error ----------------------------

/// `query notion --status active`: `--status` is not a `notion` filter, so
/// clap rejects it before any command runs, exiting 2 -- never a command
/// that silently ignores the flag.
#[test]
fn query_notion_with_an_inapplicable_status_filter_is_a_clap_usage_error() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["query", "notion", "--status", "active"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(2), "a usage error exits 2");
}

/// `query notion --triggered-by InvoiceIssued`: not a `notion` filter
/// either.
#[test]
fn query_notion_with_an_inapplicable_triggered_by_filter_is_a_clap_usage_error() {
    let tmp = with_fixture();

    let out = telos(
        tmp.path(),
        &["query", "notion", "--triggered-by", "InvoiceIssued"],
    )
    .output()
    .unwrap();

    assert_eq!(out.status.code(), Some(2), "a usage error exits 2");
}

/// `query scenario --status active`: `--status` is an intent-only filter.
#[test]
fn query_scenario_with_an_inapplicable_status_filter_is_a_clap_usage_error() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["query", "scenario", "--status", "active"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(2), "a usage error exits 2");
}

// --- query: human mode -------------------------------------------------------

/// Human mode: one line per intent, id then title after two spaces.
#[test]
fn query_intent_human_mode_prints_id_and_title() {
    let tmp = with_fixture();

    let out = telos(
        tmp.path(),
        &["query", "intent", "--using", "NOT:billing/Invoice"],
    )
    .output()
    .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim_end(),
        "INT-0017  Issuing an invoice opens it\nINT-0042  Invoice payment marks it settled"
    );
}

/// Human mode for notions is def-less: just the bare name, one per line.
#[test]
fn query_notion_human_mode_prints_bare_names() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["query", "notion", "--kind", "event"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim_end(),
        "billing/InvoiceIssued\nbilling/PaymentReceived"
    );
}

// --- impact: exact golden ----------------------------------------------------

/// `impact INT-0017`'s reverse closure, exactly: two entries at distance 1
/// -- INT-0042 requires it directly, and SCN-0091 (INT-0017's own nested
/// scenario) verifies it directly, `NodeRef`'s variant order placing the
/// intent before the scenario; SCN-0107 verifies INT-0042 and
/// `src/billing/invoice.rs` implements INT-0042, both at distance 2. The
/// canonical sealed fixture also proves SCN-0091 with `tests/billing.rs`, at
/// distance 2; the qualified proof of SCN-0107 sits one hop further out, at
/// distance 3.
///
/// The five-entry list is verified against `relation_graph` and
/// `reverse_closure`: every intent's nested scenario gets a `verifies` edge
/// straight to it, so INT-0017's own SCN-0091 is a distance-1 neighbor like
/// INT-0042.
#[test]
fn impact_int_0017_matches_the_exact_reverse_closure() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["impact", "INT-0017", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    assert_eq!(envelope["result"]["id"], json!("INT-0017"));
    assert_eq!(
        envelope["result"]["impacted"],
        json!([
            { "id": "INT-0042", "via": "requires", "distance": 1 },
            { "id": "SCN-0091", "via": "verifies", "distance": 1 },
            { "id": "SCN-0107", "via": "verifies", "distance": 2 },
            { "id": "src/billing/invoice.rs", "via": "implements", "distance": 2 },
            { "id": "tests/billing.rs", "via": "proves", "distance": 2 },
            {
                "id": "tests/billing.rs::scn_0107_full_payment_settles_the_invoice",
                "via": "proves",
                "distance": 3
            },
        ])
    );
}

/// `impact Invoice`: both intents that use it directly, and both scenarios
/// that use it directly (one in `given`, one in `then`), all sit at
/// distance 1 via `uses`.
#[test]
fn impact_invoice_contains_the_intent_and_scenario_that_use_it_directly() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["impact", "NOT:billing/Invoice", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let envelope = json_stdout(&out);
    let impacted = envelope["result"]["impacted"]
        .as_array()
        .expect("impacted is an array");
    assert!(
        impacted.contains(&json!({ "id": "INT-0042", "via": "uses", "distance": 1 })),
        "impacted: {impacted:?}"
    );
    assert!(
        impacted.contains(&json!({ "id": "SCN-0107", "via": "uses", "distance": 1 })),
        "impacted: {impacted:?}"
    );
}

// --- impact: unresolved references ------------------------------------------

/// An intent id absent from the spec: the hint suggests the nearest
/// *existing* intent id by numeric distance, exactly as `show` does --
/// `impact` reuses the same suggestion helpers.
#[test]
fn impact_unknown_intent_reports_the_numerically_closest_id() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["impact", "INT-9999", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    let hint = envelope["error"]["hint"].as_str().unwrap();
    assert!(hint.contains("INT-0042"), "hint: {hint}");
}

/// An unknown notion name close enough to an existing one: the edit-distance
/// suggestion path, distinct from the numeric one above.
#[test]
fn impact_unknown_notion_reports_the_edit_distance_closest_name() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["impact", "NOT:billing/Invoic", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    assert_eq!(
        envelope["error"]["hint"],
        json!("closest is `billing/Invoice`")
    );
}

/// An argument that is neither a typed id nor a valid (PascalCase) notion
/// name: `impact`'s own diagnosis, reused from `show`.
#[test]
fn impact_an_unparseable_argument_reports_telos_reference_unknown() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["impact", "foo-bar", "--json"])
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1), "a domain error exits 1");
    let envelope = json_stdout(&out);
    assert_eq!(envelope["error"]["code"], json!("TELOS_REFERENCE_UNKNOWN"));
    let message = envelope["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("cannot parse `foo-bar`"),
        "message: {message}"
    );
    assert_eq!(envelope["error"]["hint"], Value::Null);
}

// --- impact: human mode -------------------------------------------------------

/// Human mode: one line per entry, `<id>  (via <rel>, distance <n>)`.
#[test]
fn impact_human_mode_prints_one_line_per_entry() {
    let tmp = with_fixture();

    let out = telos(tmp.path(), &["impact", "INT-0017"]).output().unwrap();

    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("  INT-0042  (via requires, distance 1)"),
        "stdout: {stdout}"
    );
}
