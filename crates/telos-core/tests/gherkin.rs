//! Gherkin rendering: one intent becomes one `.feature`, deterministically.
//!
//! The billing corpus is the specification of the output. Its two intents
//! cover the shapes that matter: a `when` step with no fields, a `given`
//! with two fields including a money literal, and an enum-symbol `then`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use telos_core::gherkin::render_features;
use telos_core::ids::RepoPath;
use telos_core::model::TelosModel;
use telos_core::semantic::build_model;
use telos_core::workspace::parse_spec_file;

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/billing")
}

/// Every `.tel` file of the corpus, folded into a validated model.
fn corpus_model() -> TelosModel {
    let root = corpus_root();
    let mut files = Vec::new();
    let mut stack = vec![root.join("telos")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("the corpus tree is readable") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "tel") {
                let rel = path
                    .strip_prefix(&root)
                    .expect("under the corpus root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let src = fs::read_to_string(&path).expect("a readable file");
                let repo_path = RepoPath::new(rel);
                let parsed = parse_spec_file(&repo_path, &src)
                    .unwrap_or_else(|d| panic!("{repo_path} does not parse: {d:?}"));
                files.push((repo_path, parsed));
            }
        }
    }
    files.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
    build_model(files).expect("the corpus is a clean model")
}

fn rendered() -> BTreeMap<RepoPath, String> {
    render_features(&corpus_model())
}

#[test]
fn one_intent_renders_to_one_feature_file_at_its_owner_path() {
    let features = rendered();
    let paths: Vec<&str> = features.keys().map(RepoPath::as_str).collect();
    assert_eq!(
        paths,
        vec![
            "telos/features/billing/invoicing/INT-0017.feature",
            "telos/features/billing/settlement/INT-0042.feature",
        ]
    );
}

#[test]
fn a_feature_carries_the_intents_title_telos_and_id_tag() {
    let features = rendered();
    let actual = &features[&RepoPath::new("telos/features/billing/settlement/INT-0042.feature")];
    assert_eq!(
        actual,
        concat!(
            "@INT-0042\n",
            "Feature: Invoice payment marks it settled\n",
            "  Customers must see immediately that their debt is cleared.\n",
            "\n",
            "  @SCN-0107\n",
            "  Scenario: full payment settles the invoice\n",
            "    Given the invoice with state open and balance 120.00 EUR\n",
            "    When the payment is received with amount 120.00 EUR\n",
            "    Then the invoice state is settled\n",
        )
    );
}

#[test]
fn a_when_step_without_fields_drops_the_with_clause() {
    let features = rendered();
    let actual = &features[&RepoPath::new("telos/features/billing/invoicing/INT-0017.feature")];
    assert_eq!(
        actual,
        concat!(
            "@INT-0017\n",
            "Feature: Issuing an invoice opens it\n",
            "  An invoice must start its life open and unpaid.\n",
            "\n",
            "  @SCN-0091\n",
            "  Scenario: a newly issued invoice is open\n",
            "    Given the customer with name ACME\n",
            "    When the invoice is issued\n",
            "    Then the invoice state is open\n",
        )
    );
}

#[test]
fn rendering_is_deterministic() {
    assert_eq!(rendered(), rendered());
}

// --- expression coverage -------------------------------------------------

/// Renders INT-0042 with its `then` line replaced, and returns just the
/// step lines that follow the `When`.
///
/// The corpus exercises only `==` against an enum symbol; every other
/// `Expr` shape gets its prose pinned here.
fn then_steps(then_source: &str) -> Vec<String> {
    let root = corpus_root();
    let mut files = Vec::new();
    let mut stack = vec![root.join("telos")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("the corpus tree is readable") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "tel") {
                continue;
            }
            let rel = path
                .strip_prefix(&root)
                .expect("under the corpus root")
                .to_string_lossy()
                .replace('\\', "/");
            let mut src = fs::read_to_string(&path).expect("a readable file");
            if rel.ends_with("INT-0042.tel") {
                src = src.replace("then  Invoice.state == settled", then_source);
                assert!(
                    src.contains(then_source),
                    "the `then` line was not replaced"
                );
            }
            let repo_path = RepoPath::new(rel);
            let parsed = parse_spec_file(&repo_path, &src)
                .unwrap_or_else(|d| panic!("{repo_path} does not parse: {d:?}"));
            files.push((repo_path, parsed));
        }
    }
    files.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
    let model = build_model(files).expect("a clean model");
    let feature = render_features(&model)
        [&RepoPath::new("telos/features/billing/settlement/INT-0042.feature")]
        .clone();
    feature
        .lines()
        .skip_while(|line| !line.starts_with("    When "))
        .skip(1)
        // Indentation is pinned byte-exactly by the feature tests above; these
        // assertions are about the keyword and the prose.
        .map(|line| line.trim_start().to_string())
        .collect()
}

#[test]
fn every_comparison_operator_renders_as_a_verb() {
    for (source, expected) in [
        (
            "then  Invoice.state == settled",
            "Then the invoice state is settled",
        ),
        (
            "then  Invoice.state != settled",
            "Then the invoice state is not settled",
        ),
        (
            "then  Invoice.balance < \"1.00 EUR\"",
            "Then the invoice balance is less than 1.00 EUR",
        ),
        (
            "then  Invoice.balance <= \"1.00 EUR\"",
            "Then the invoice balance is at most 1.00 EUR",
        ),
        (
            "then  Invoice.balance > \"1.00 EUR\"",
            "Then the invoice balance is greater than 1.00 EUR",
        ),
        (
            "then  Invoice.balance >= \"1.00 EUR\"",
            "Then the invoice balance is at least 1.00 EUR",
        ),
    ] {
        assert_eq!(then_steps(source), vec![expected.to_string()], "{source}");
    }
}

#[test]
fn an_in_set_renders_as_one_of() {
    assert_eq!(
        then_steps("then  Invoice.state in (open, settled, cancelled)"),
        vec!["Then the invoice state is one of open, settled and cancelled".to_string()]
    );
}

#[test]
fn an_and_becomes_a_second_step_but_an_or_stays_in_one() {
    assert_eq!(
        then_steps("then  Invoice.state == settled and Invoice.balance == \"0.00 EUR\""),
        vec![
            "Then the invoice state is settled".to_string(),
            "And the invoice balance is 0.00 EUR".to_string(),
        ]
    );
    assert_eq!(
        then_steps("then  Invoice.state == settled or Invoice.state == cancelled"),
        vec!["Then the invoice state is settled or the invoice state is cancelled".to_string()]
    );
}

#[test]
fn a_negation_says_so_in_words() {
    assert_eq!(
        then_steps("then  not Invoice.state == open"),
        vec!["Then it is not the case that the invoice state is open".to_string()]
    );
}

#[test]
fn several_then_lines_become_then_and_and() {
    assert_eq!(
        then_steps(concat!(
            "then  Invoice.state == settled\n",
            "    then  Invoice.balance == \"0.00 EUR\""
        )),
        vec![
            "Then the invoice state is settled".to_string(),
            "And the invoice balance is 0.00 EUR".to_string(),
        ]
    );
}
