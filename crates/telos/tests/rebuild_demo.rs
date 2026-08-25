//! Public acceptance proof for rebuilding the spec-only Billing demo.
//!
//! This harness is deliberately an external implementer: it copies the public
//! demo into fresh git repositories, invokes only the real `telos` binary, and
//! writes only the generated Cargo application. It never edits `telos/` or
//! calls a core API.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use common::{repo, telos};

const INT_0017: &str = "INT-0017";
const INT_0042: &str = "INT-0042";
const SCN_0091: &str = "SCN-0091";
const SCN_0107: &str = "SCN-0107";
const CARGO_LOCK: &str = "Cargo.lock";
const CARGO_MANIFEST: &str = "Cargo.toml";
const CODE: &str = "src/lib.rs";
const ISSUED_TEST: &str = "tests/invoice_issued.rs";
const ISSUED_FN: &str = "scn_0091_new_invoice_is_open";
const PAYMENT_TEST: &str = "tests/payment_received.rs";
const PAYMENT_FN: &str = "scn_0107_full_payment_settles_invoice";
const CONSTRAINT: &str = "CON-0003";
const CONSTRAINT_CHECK: &str =
    "cargo test --test invoice_issued domain_does_not_import_adapter_modules -- --exact";

// These bytes model an external implementer. They deliberately live in the
// conformance harness, never in the public spec-only demo tree.
const APPLICATION_MANIFEST: &str = r#"[package]
name = "billing-rebuild"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"

[dev-dependencies]
syn = { version = "=3.0.3", features = ["full", "visit"] }
"#;

const RED_SOURCE: &str = "// The scenario test must fail before the first implementation exists.\n";

const INVOICE_ISSUED_TEST: &str = r#"use std::fs;
use std::path::{Path, PathBuf};

use billing_rebuild::{Invoice, InvoiceState};
use syn::visit::Visit;
use syn::{ItemUse, UseTree};

#[test]
fn scn_0091_new_invoice_is_open() {
    let invoice = Invoice::issued_to("ACME", 12_000);
    assert_eq!(invoice.state(), InvoiceState::Open);
}

fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("read source entry").path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn is_adapter_path(path: &[String]) -> bool {
    path.first().is_some_and(|segment| segment == "adapters")
        || path.windows(2).any(|segments| {
            matches!(segments[0].as_str(), "crate" | "self" | "super")
                && segments[1] == "adapters"
        })
}

fn imports_adapters(tree: &UseTree, path: &mut Vec<String>) -> bool {
    match tree {
        UseTree::Path(node) => {
            path.push(node.ident.to_string());
            let forbidden = is_adapter_path(path) || imports_adapters(&node.tree, path);
            path.pop();
            forbidden
        }
        UseTree::Name(node) => {
            path.push(node.ident.to_string());
            let forbidden = is_adapter_path(path);
            path.pop();
            forbidden
        }
        UseTree::Rename(node) => {
            path.push(node.ident.to_string());
            let forbidden = is_adapter_path(path);
            path.pop();
            forbidden
        }
        UseTree::Glob(_) => is_adapter_path(path),
        UseTree::Group(group) => group
            .items
            .iter()
            .any(|tree| imports_adapters(tree, path)),
    }
}

#[derive(Default)]
struct LayerVisitor {
    adapter_imports: usize,
}

impl<'ast> Visit<'ast> for LayerVisitor {
    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        if imports_adapters(&item.tree, &mut Vec::new()) {
            self.adapter_imports += 1;
        }
    }
}

#[test]
fn domain_does_not_import_adapter_modules() {
    let mut files = Vec::new();
    rust_files(Path::new("src"), &mut files);
    files.sort();
    for path in files {
        let source = fs::read_to_string(&path).expect("read domain source");
        let syntax = syn::parse_file(&source).expect("parse domain source");
        let mut visitor = LayerVisitor::default();
        visitor.visit_file(&syntax);
        assert_eq!(
            visitor.adapter_imports,
            0,
            "{} imports the adapters layer",
            path.display()
        );
    }
}
"#;

const FIRST_IMPLEMENTATION: &str = r#"#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvoiceState {
    Open,
    Settled,
}

pub mod adapters_v2 {
    pub struct LedgerAdapterV2;
}

// Documentation only: `use crate::adapters::LedgerAdapter;` is forbidden.
const FORBIDDEN_IMPORT_EXAMPLE: &str = "use crate::adapters::LedgerAdapter;";

pub struct Invoice {
    customer: String,
    balance_cents: u64,
    state: InvoiceState,
}

impl Invoice {
    pub fn issued_to(customer: &str, balance_cents: u64) -> Self {
        Self {
            customer: customer.to_owned(),
            balance_cents,
            state: InvoiceState::Open,
        }
    }

    pub fn state(&self) -> InvoiceState {
        self.state
    }

    pub fn customer(&self) -> &str {
        &self.customer
    }

    pub fn balance_cents(&self) -> u64 {
        self.balance_cents
    }

    pub fn harmless_adapter_references(&self) -> (&'static str, &'static str) {
        (
            FORBIDDEN_IMPORT_EXAMPLE,
            std::any::type_name::<adapters_v2::LedgerAdapterV2>(),
        )
    }
}
"#;

const PAYMENT_RECEIVED_TEST: &str = r#"use billing_rebuild::{Invoice, InvoiceState};

#[test]
fn scn_0107_full_payment_settles_invoice() {
    let mut invoice = Invoice::issued_to("ACME", 12_000);
    invoice.receive_payment(12_000);
    assert_eq!(invoice.state(), InvoiceState::Settled);
}
"#;

const CONSTRAINT_VIOLATING_IMPLEMENTATION: &str = r#"pub mod adapters {
    pub struct LedgerAdapter;
    pub struct MailAdapter;
}

use crate :: adapters::LedgerAdapter;
use crate::{adapters::MailAdapter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvoiceState {
    Open,
    Settled,
}

pub struct Invoice {
    customer: String,
    balance_cents: u64,
    state: InvoiceState,
}

impl Invoice {
    pub fn issued_to(customer: &str, balance_cents: u64) -> Self {
        Self {
            customer: customer.to_owned(),
            balance_cents,
            state: InvoiceState::Open,
        }
    }

    pub fn receive_payment(&mut self, amount_cents: u64) {
        if amount_cents >= self.balance_cents {
            self.balance_cents = 0;
            self.state = InvoiceState::Settled;
        }
    }

    pub fn state(&self) -> InvoiceState {
        self.state
    }

    pub fn customer(&self) -> &str {
        &self.customer
    }

    pub fn balance_cents(&self) -> u64 {
        self.balance_cents
    }

    pub fn adapter_type_names(&self) -> (&'static str, &'static str) {
        (
            std::any::type_name::<LedgerAdapter>(),
            std::any::type_name::<MailAdapter>(),
        )
    }
}
"#;

const FINAL_IMPLEMENTATION: &str = r#"#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvoiceState {
    Open,
    Settled,
}

pub struct Invoice {
    customer: String,
    balance_cents: u64,
    state: InvoiceState,
}

impl Invoice {
    pub fn issued_to(customer: &str, balance_cents: u64) -> Self {
        Self {
            customer: customer.to_owned(),
            balance_cents,
            state: InvoiceState::Open,
        }
    }

    pub fn receive_payment(&mut self, amount_cents: u64) {
        if amount_cents >= self.balance_cents {
            self.balance_cents = 0;
            self.state = InvoiceState::Settled;
        }
    }

    pub fn state(&self) -> InvoiceState {
        self.state
    }

    pub fn customer(&self) -> &str {
        &self.customer
    }

    pub fn balance_cents(&self) -> u64 {
        self.balance_cents
    }
}
"#;

#[derive(Debug, PartialEq)]
struct Observations {
    plan: Value,
    initial_status: Value,
    red_runs: Vec<Value>,
    green_runs: Vec<Value>,
    progress: Vec<Value>,
    final_status: Value,
}

fn demo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo/billing")
}

fn documented_block(root: &Path, marker: &str, language: &str) -> String {
    let readme = fs::read_to_string(root.join("README.md")).expect("read demo README");
    let start = format!("<!-- {marker}:start -->\n```{language}\n");
    let end = format!("\n```\n<!-- {marker}:end -->");
    let (_, after_start) = readme
        .split_once(&start)
        .unwrap_or_else(|| panic!("README is missing executable `{marker}` start marker"));
    let (block, _) = after_start
        .split_once(&end)
        .unwrap_or_else(|| panic!("README is missing executable `{marker}` end marker"));
    format!("{block}\n")
}

fn documented_json(root: &Path, marker: &str) -> Value {
    serde_json::from_str(&documented_block(root, marker, "json"))
        .unwrap_or_else(|error| panic!("README `{marker}` is not valid JSON: {error}"))
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap_or_else(|error| panic!("mkdir {}: {error}", dst.display()));
    for entry in
        fs::read_dir(src).unwrap_or_else(|error| panic!("read_dir {}: {error}", src.display()))
    {
        let entry = entry.expect("read demo entry");
        let target = dst.join(entry.file_name());
        if entry.file_type().expect("stat demo entry").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target)
                .unwrap_or_else(|error| panic!("copy {}: {error}", entry.path().display()));
        }
    }
}

fn fresh_demo() -> tempfile::TempDir {
    let tmp = repo();
    copy_dir(&demo_root(), tmp.path());
    tmp
}

fn execute(
    root: &Path,
    target_dir: &Path,
    args: &[&str],
    stdin: Option<&str>,
) -> (std::process::ExitStatus, Value, String) {
    let mut command = telos(root, args);
    command.env("CARGO_TARGET_DIR", target_dir);
    command.env("CARGO_NET_OFFLINE", "true");
    if let Some(stdin) = stdin {
        command.write_stdin(stdin.to_owned());
    }
    let output = command.output().expect("run public telos binary");
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "telos {} did not return JSON ({error})\nstdout: {}\nstderr: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
    });
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (output.status, envelope, stderr)
}

fn run(root: &Path, target_dir: &Path, args: &[&str], stdin: Option<&str>) -> Value {
    let (status, envelope, stderr) = execute(root, target_dir, args, stdin);
    assert!(
        status.success(),
        "telos {} failed: {envelope:#}\nstderr: {stderr}",
        args.join(" ")
    );
    assert_eq!(envelope["ok"], json!(true));
    envelope
}

fn error(root: &Path, target_dir: &Path, args: &[&str]) -> Value {
    let (status, envelope, stderr) = execute(root, target_dir, args, None);
    assert!(
        !status.success(),
        "telos {} unexpectedly succeeded: {envelope:#}\nstderr: {stderr}",
        args.join(" ")
    );
    assert_eq!(envelope["ok"], json!(false));
    envelope["error"].clone()
}

fn result(root: &Path, target_dir: &Path, args: &[&str]) -> Value {
    run(root, target_dir, args, None)["result"].clone()
}

fn result_stdin(root: &Path, target_dir: &Path, args: &[&str], stdin: &Value) -> Value {
    run(root, target_dir, args, Some(&stdin.to_string()))["result"].clone()
}

fn is_digest(value: &Value) -> bool {
    value.as_str().is_some_and(|digest| {
        digest
            .strip_prefix("sha256:")
            .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
    })
}

struct Batch<'a> {
    intent: &'a str,
    scenario: &'a str,
    test_path: &'a str,
    test_name: &'a str,
    test_bytes: &'a str,
    implementation_bytes: &'a str,
    expected_green: u64,
}

fn implement_batch(
    root: &Path,
    target_dir: &Path,
    batch: Batch<'_>,
) -> (Value, Value, Value, Value) {
    let opened = result(
        root,
        target_dir,
        &[
            "change",
            "open",
            &format!("rebuild {}", batch.intent),
            "--json",
        ],
    );
    let change = opened["id"].as_str().expect("change id").to_owned();

    let staged = result_stdin(
        root,
        target_dir,
        &[
            "edit",
            "intent",
            batch.intent,
            "--change",
            &change,
            "--json",
        ],
        &documented_json(root, "intent-activation"),
    );
    assert_eq!(staged["claims"], json!([intent_spec_path(batch.intent)]));
    assert_eq!(staged["scenario_ids"], json!([]));

    if batch.intent == INT_0017 {
        let staged_constraint = result_stdin(
            root,
            target_dir,
            &[
                "edit",
                "constraint",
                CONSTRAINT,
                "--change",
                &change,
                "--json",
            ],
            &documented_json(root, "constraint-check-patch"),
        );
        assert_eq!(
            staged_constraint["claims"],
            json!([
                intent_spec_path(batch.intent),
                format!("telos/contexts/billing/constraints/{CONSTRAINT}.tel")
            ])
        );
    }

    let diff = result(root, target_dir, &["change", "diff", &change, "--json"]);
    assert!(
        is_digest(&diff["digest"]),
        "non-empty operation digest: {diff:#}"
    );
    let ops = diff["ops"].as_array().expect("diff ops");
    assert_eq!(ops.len(), if batch.intent == INT_0017 { 2 } else { 1 });
    let intent_op = ops
        .iter()
        .find(|op| op["entity"] == "intent" && op["key"] == batch.intent)
        .expect("intent activation op");
    assert_eq!(intent_op["op"], json!("edit"));
    let before = intent_op["before"].as_str().expect("intent before");
    let after = intent_op["after"].as_str().expect("intent after");
    assert!(before.contains("  status draft\n"), "{before}");
    assert!(!before.contains("  status active\n"), "{before}");
    assert!(after.contains("  status active\n"), "{after}");
    assert!(!after.contains("  status draft\n"), "{after}");
    assert_ne!(
        before, after,
        "activation must be a real draft -> active delta"
    );

    if batch.intent == INT_0017 {
        let constraint_op = ops
            .iter()
            .find(|op| op["entity"] == "constraint" && op["key"] == CONSTRAINT)
            .expect("constraint check op");
        assert_eq!(constraint_op["op"], json!("edit"));
        let before = constraint_op["before"].as_str().expect("constraint before");
        let after = constraint_op["after"].as_str().expect("constraint after");
        assert!(!before.contains("  check \""), "{before}");
        assert!(
            after.contains(&format!("  check \"{CONSTRAINT_CHECK}\"\n")),
            "{after}"
        );
        assert_ne!(before, after, "the machine check must be staged");
    }

    let digest = diff["digest"].as_str().expect("diff digest");
    let approved = result(
        root,
        target_dir,
        &[
            "change",
            "approve",
            &change,
            "--expected-digest",
            digest,
            "--json",
        ],
    );
    assert_eq!(approved["digest"], diff["digest"]);

    let progress_before = result(root, target_dir, &["rebuild", "status", "--json"]);
    assert_eq!(
        progress_before["scenarios_green"],
        json!(batch.expected_green - 1)
    );
    assert_eq!(progress_before["scenarios_total"], json!(2));

    if batch.intent == INT_0017 {
        fs::write(root.join(CARGO_MANIFEST), APPLICATION_MANIFEST).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join(CODE), RED_SOURCE).unwrap();
    }
    fs::create_dir_all(root.join("tests")).unwrap();
    fs::write(root.join(batch.test_path), batch.test_bytes).unwrap();
    let test_bytes = fs::read(root.join(batch.test_path)).unwrap();

    let red = result(root, target_dir, &["test", batch.scenario, "--json"]);
    assert_eq!(red["witness"], json!("red"));
    assert_eq!(
        red["test"],
        json!(format!("{}::{}", batch.test_path, batch.test_name))
    );
    if batch.intent == INT_0017 {
        assert!(root.join(CARGO_LOCK).exists(), "Cargo generated its lock");
    }

    fs::write(root.join(CODE), batch.implementation_bytes).unwrap();
    let paths = if batch.intent == INT_0017 {
        vec![CARGO_LOCK, CARGO_MANIFEST, CODE]
    } else {
        vec![CODE]
    };
    for path in paths {
        let bound = result(root, target_dir, &["bind", path, batch.intent, "--json"]);
        assert_eq!(bound["change"], json!(change));
        assert_eq!(bound["path"], json!(path));
        assert_eq!(bound["intent"], json!(batch.intent));
    }
    assert_eq!(
        fs::read(root.join(batch.test_path)).unwrap(),
        test_bytes,
        "red and green must run on identical test bytes"
    );

    let green = result(root, target_dir, &["test", batch.scenario, "--json"]);
    assert_eq!(green["witness"], json!("green"));
    assert_eq!(green["test"], red["test"]);

    if batch.intent == INT_0042 {
        let constraint_error = error(
            root,
            target_dir,
            &["change", "reconcile", &change, "--json"],
        );
        assert_eq!(
            constraint_error,
            json!({
                "code": "TELOS_CONSTRAINT_FAILED",
                "message": format!("{CONSTRAINT} check failed: `{CONSTRAINT_CHECK}`"),
                "hint": "Run the constraint's `check` command directly to see its output."
            })
        );
        assert!(root.join(format!("telos/changes/{change}.tel")).exists());
        fs::write(root.join(CODE), FINAL_IMPLEMENTATION).unwrap();
    }

    let reconciled = result(
        root,
        target_dir,
        &["change", "reconcile", &change, "--json"],
    );
    assert_eq!(reconciled["id"], json!(change));
    assert_eq!(
        reconciled["ops_applied"],
        json!(if batch.intent == INT_0017 { 2 } else { 1 })
    );
    assert_eq!(reconciled["tests_run"], json!(1));
    assert_eq!(reconciled["checks_run"], json!(1));
    assert!(!root.join(format!("telos/changes/{change}.tel")).exists());

    let progress = result(root, target_dir, &["rebuild", "status", "--json"]);
    assert_eq!(progress["scenarios_green"], json!(batch.expected_green));
    assert_eq!(progress["scenarios_total"], json!(2));

    (progress_before, red, green, progress)
}

fn intent_spec_path(intent: &str) -> String {
    let capability = if intent == INT_0017 {
        "invoicing"
    } else {
        "settlement"
    };
    format!("telos/contexts/billing/capabilities/{capability}/intents/{intent}.tel")
}

fn assert_initial_plan(plan: &Value) {
    let steps = plan["steps"].as_array().expect("plan steps");
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["n"], json!(1));
    assert_eq!(steps[0]["intent"], json!(INT_0017));
    assert_eq!(steps[0]["requires"], json!([]));
    assert_eq!(steps[1]["n"], json!(2));
    assert_eq!(steps[1]["intent"], json!(INT_0042));
    assert_eq!(steps[1]["requires"], json!([INT_0017]));

    let full_pack_keys = BTreeSet::from([
        "bindings",
        "canonical",
        "change",
        "constraints",
        "id",
        "mappings",
        "neighbors",
        "notions",
        "owner",
        "scenarios",
    ]);
    for step in steps {
        assert_eq!(
            step["pack"]
                .as_object()
                .expect("full work pack")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            full_pack_keys
        );
    }

    assert_eq!(steps[0]["pack"]["id"], json!(INT_0017));
    assert_eq!(
        steps[0]["pack"]["owner"],
        json!({"context": "billing", "capability": "invoicing"})
    );
    assert_eq!(steps[0]["pack"]["change"], Value::Null);
    assert!(
        steps[0]["pack"]["canonical"]
            .as_str()
            .expect("canonical first intent")
            .contains("  status draft\n")
    );
    assert_eq!(
        steps[0]["pack"]["scenarios"],
        json!([{
            "id": SCN_0091,
            "title": "a newly issued invoice is open",
            "proved": false
        }])
    );
    assert_eq!(
        steps[0]["pack"]["notions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|notion| notion["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "billing/Customer",
            "billing/Invoice",
            "billing/InvoiceIssued"
        ]
    );
    assert_eq!(
        steps[0]["pack"]["bindings"],
        json!({"implements": [], "proves": []})
    );
    assert_eq!(steps[0]["pack"]["constraints"][0]["id"], json!("CON-0003"));
    assert_eq!(
        steps[0]["pack"]["constraints"][0]["scope"],
        json!("context")
    );
    assert!(
        steps[0]["pack"]["canonical"]
            .as_str()
            .unwrap()
            .contains("system shall set Invoice.state = open")
    );
    assert_eq!(
        steps[0]["pack"]["neighbors"],
        json!([{
            "id": INT_0042,
            "title": "Invoice payment marks it settled",
            "rel": "requires",
            "direction": "in"
        }])
    );

    assert_eq!(steps[1]["pack"]["id"], json!(INT_0042));
    assert_eq!(
        steps[1]["pack"]["owner"],
        json!({"context": "billing", "capability": "settlement"})
    );
    assert_eq!(steps[1]["pack"]["change"], Value::Null);
    assert!(
        steps[1]["pack"]["canonical"]
            .as_str()
            .expect("canonical second intent")
            .contains("  status draft\n")
    );
    assert_eq!(
        steps[1]["pack"]["scenarios"],
        json!([{
            "id": SCN_0107,
            "title": "full payment settles the invoice",
            "proved": false
        }])
    );
    assert_eq!(
        steps[1]["pack"]["notions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|notion| notion["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["billing/Invoice", "billing/PaymentReceived"]
    );
    assert_eq!(
        steps[1]["pack"]["bindings"],
        json!({"implements": [], "proves": []})
    );
    assert_eq!(steps[1]["pack"]["constraints"][0]["id"], json!("CON-0003"));
    assert_eq!(
        steps[1]["pack"]["constraints"][0]["scope"],
        json!("context")
    );
    assert!(
        steps[1]["pack"]["canonical"]
            .as_str()
            .unwrap()
            .contains("system shall set Invoice.state = settled")
    );
    assert_eq!(
        steps[1]["pack"]["neighbors"],
        json!([{
            "id": INT_0017,
            "title": "Issuing an invoice opens it",
            "rel": "requires",
            "direction": "out"
        }])
    );
}

fn relative_files(root: &Path) -> BTreeSet<String> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).expect("read reconstructed directory") {
            let entry = entry.expect("read reconstructed entry");
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap();
            if relative.starts_with(".git") {
                continue;
            }
            if entry
                .file_type()
                .expect("stat reconstructed entry")
                .is_dir()
            {
                visit(root, &path, files);
            } else {
                files.insert(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }

    let mut files = BTreeSet::new();
    visit(root, root, &mut files);
    files
}

fn assert_public_demo_is_solution_free(root: &Path) {
    let expected = BTreeSet::from([
        "README.md".to_owned(),
        "telos/context-map.tel".to_owned(),
        "telos/contexts/billing/context.tel".to_owned(),
        "telos/contexts/billing/bindings.tel".to_owned(),
        "telos/contexts/billing/constraints/CON-0003.tel".to_owned(),
        "telos/contexts/billing/capabilities/invoicing/capability.tel".to_owned(),
        "telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel".to_owned(),
        "telos/contexts/billing/capabilities/settlement/capability.tel".to_owned(),
        "telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel".to_owned(),
        "telos/contexts/billing/notions/Customer.tel".to_owned(),
        "telos/contexts/billing/notions/Invoice.tel".to_owned(),
        "telos/contexts/billing/capabilities/invoicing/notions/InvoiceIssued.tel".to_owned(),
        "telos/contexts/billing/capabilities/settlement/notions/PaymentReceived.tel".to_owned(),
        "telos/telos.toml".to_owned(),
    ]);
    assert_eq!(
        relative_files(root),
        expected,
        "the public demo must contain only documentation and Telos spec owners"
    );

    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    for solution_fragment in [
        "name = \"billing-rebuild\"",
        "pub struct Invoice",
        "use billing_rebuild::",
        "fn scn_0091_new_invoice_is_open",
        "fn scn_0107_full_payment_settles_invoice",
        "cat > Cargo.toml",
        "cat > src/",
        "cat > tests/",
    ] {
        assert!(
            !readme.contains(solution_fragment),
            "README leaks fixture bytes through `{solution_fragment}`"
        );
    }
}

fn reconstruct(target_dir: &Path) -> Observations {
    let tmp = fresh_demo();
    let root = tmp.path();

    assert_public_demo_is_solution_free(root);

    assert!(!root.join("Cargo.toml").exists());
    assert!(!root.join("Cargo.lock").exists());
    assert!(!root.join("src").exists());
    assert!(!root.join("tests").exists());
    assert!(!root.join("architecture").exists());
    assert!(!root.join("telos/telos.lock").exists());
    assert_eq!(
        fs::read_to_string(root.join("telos/contexts/billing/bindings.tel")).unwrap(),
        ""
    );

    let plan = result(root, target_dir, &["rebuild", "plan", "--json"]);
    assert_initial_plan(&plan);
    let initial_status = result(root, target_dir, &["rebuild", "status", "--json"]);
    assert_eq!(initial_status["scenarios_green"], json!(0));
    assert_eq!(initial_status["scenarios_total"], json!(2));
    assert_eq!(initial_status["scenarios"][0]["tests"], json!([]));
    assert_eq!(initial_status["scenarios"][1]["tests"], json!([]));
    let bootstrapped = result(
        root,
        target_dir,
        &["change", "reconcile", "--full", "--json"],
    );
    assert_eq!(bootstrapped["ops_applied"], json!(0));
    assert_eq!(bootstrapped["checks_run"], json!(0));
    assert_eq!(bootstrapped["tests_run"], json!(0));
    assert!(root.join("telos/telos.lock").exists());
    assert!(!root.join(CARGO_MANIFEST).exists());
    assert!(!root.join(CARGO_LOCK).exists());
    assert!(!root.join("src").exists());
    assert!(!root.join("tests").exists());
    assert!(!root.join("architecture").exists());
    let status = result(root, target_dir, &["status", "--json"]);
    assert_eq!(status["state"], json!("coherent"));
    assert_eq!(status["coverage"]["scenarios_proved"], json!(0));

    let first = implement_batch(
        root,
        target_dir,
        Batch {
            intent: INT_0017,
            scenario: SCN_0091,
            test_path: ISSUED_TEST,
            test_name: ISSUED_FN,
            test_bytes: INVOICE_ISSUED_TEST,
            implementation_bytes: FIRST_IMPLEMENTATION,
            expected_green: 1,
        },
    );
    assert_eq!(first.0, initial_status);
    assert_eq!(first.3["scenarios"][0]["green"], json!(true));
    assert_eq!(first.3["scenarios"][1]["green"], json!(false));

    let second = implement_batch(
        root,
        target_dir,
        Batch {
            intent: INT_0042,
            scenario: SCN_0107,
            test_path: PAYMENT_TEST,
            test_name: PAYMENT_FN,
            test_bytes: PAYMENT_RECEIVED_TEST,
            implementation_bytes: CONSTRAINT_VIOLATING_IMPLEMENTATION,
            expected_green: 2,
        },
    );

    let sealed = result(root, target_dir, &["check", "--sealed", "--json"]);
    assert_eq!(sealed["diagnostics"], json!([]));
    let project_status = result(root, target_dir, &["status", "--json"]);
    assert_eq!(project_status["state"], json!("coherent"));
    assert_eq!(project_status["changes"], json!([]));
    let final_status = result(root, target_dir, &["rebuild", "status", "--json"]);
    assert_eq!(final_status["scenarios_green"], json!(2));
    assert_eq!(final_status["scenarios_total"], json!(2));
    assert!(
        final_status["scenarios"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["green"] == json!(true))
    );
    assert_eq!(
        fs::read_to_string(root.join(format!(
            "telos/contexts/billing/constraints/{CONSTRAINT}.tel"
        )))
        .unwrap(),
        format!(
            "constraint {CONSTRAINT} in context billing architecture \"Hexagonal boundaries\" {{\n  \
             rule  \"Domain code must not import adapter modules.\"\n  \
             check \"{CONSTRAINT_CHECK}\"\n\
             }}\n"
        )
    );

    let bindings = fs::read_to_string(root.join("telos/contexts/billing/bindings.tel")).unwrap();
    assert_eq!(
        bindings,
        "implements \"Cargo.lock\" -> INT-0017\n\
         implements \"Cargo.toml\" -> INT-0017\n\
         implements \"src/lib.rs\" -> INT-0017\n\
         implements \"src/lib.rs\" -> INT-0042\n\
         proves     \"tests/invoice_issued.rs::scn_0091_new_invoice_is_open\" -> SCN-0091\n\
         proves     \"tests/payment_received.rs::scn_0107_full_payment_settles_invoice\" -> SCN-0107\n"
    );
    let lock = fs::read_to_string(root.join("telos/telos.lock")).unwrap();
    for path in [
        CARGO_LOCK,
        CARGO_MANIFEST,
        CODE,
        ISSUED_TEST,
        PAYMENT_TEST,
        "telos/contexts/billing/constraints/CON-0003.tel",
        "telos/contexts/billing/context.tel",
        "telos/context-map.tel",
        "telos/telos.toml",
    ] {
        assert!(lock.contains(path), "`{path}` is not sealed:\n{lock}");
    }
    assert!(
        fs::read_dir(root.join("telos/changes"))
            .unwrap()
            .all(|entry| { entry.unwrap().file_name() == "counters.toml" })
    );

    let expected_files = BTreeSet::from([
        "Cargo.lock".to_owned(),
        "Cargo.toml".to_owned(),
        "README.md".to_owned(),
        CODE.to_owned(),
        ISSUED_TEST.to_owned(),
        PAYMENT_TEST.to_owned(),
        "telos/context-map.tel".to_owned(),
        "telos/contexts/billing/context.tel".to_owned(),
        "telos/contexts/billing/bindings.tel".to_owned(),
        "telos/changes/counters.toml".to_owned(),
        "telos/contexts/billing/constraints/CON-0003.tel".to_owned(),
        "telos/contexts/billing/capabilities/invoicing/capability.tel".to_owned(),
        "telos/contexts/billing/capabilities/invoicing/intents/INT-0017.tel".to_owned(),
        "telos/contexts/billing/capabilities/settlement/capability.tel".to_owned(),
        "telos/contexts/billing/capabilities/settlement/intents/INT-0042.tel".to_owned(),
        "telos/contexts/billing/notions/Customer.tel".to_owned(),
        "telos/contexts/billing/notions/Invoice.tel".to_owned(),
        "telos/contexts/billing/capabilities/invoicing/notions/InvoiceIssued.tel".to_owned(),
        "telos/contexts/billing/capabilities/settlement/notions/PaymentReceived.tel".to_owned(),
        "telos/telos.lock".to_owned(),
        "telos/telos.toml".to_owned(),
    ]);
    assert_eq!(relative_files(root), expected_files);

    Observations {
        plan,
        initial_status,
        red_runs: vec![first.1, second.1],
        green_runs: vec![first.2, second.2],
        progress: vec![first.3, second.3],
        final_status,
    }
}

#[test]
fn public_billing_demo_contains_no_extractable_solution() {
    assert_public_demo_is_solution_free(&demo_root());
}

#[test]
fn spec_only_billing_rebuilds_twice_through_the_public_cli() {
    let target = tempfile::tempdir().expect("temporary Cargo target directory");
    let first = reconstruct(target.path());
    let second = reconstruct(target.path());

    assert_eq!(
        second, first,
        "fresh reconstructions must have the same observations"
    );
}
