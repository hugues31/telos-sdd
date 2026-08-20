# Billing reconstruction demo

This directory is intentionally a spec-only Telos project. It ships no
`Cargo.toml`, lock, application source, test, generated checker, site, build
artifact, or hidden solution. Both intents start as `draft`, and the initial
architecture constraint is declarative. Telos supplies the ordered bounded
plan and verifies the result; an external `telos-implementer` writes the
application without ever writing below `telos/`.

Inspect the prerequisite-first plan:

```console
telos rebuild plan --json
```

Bootstrap the untouched Telos-only tree. With no active behavioral obligation
and no machine constraint yet, this honest first seal runs zero tests and zero
checks and leaves progress at `0/2`:

```console
telos change reconcile --full --json
telos status --json
```

## Batch 1: activate invoice issuance

Open the first batch and stage the real `draft` to `active` transition:

<!-- intent-activation:start -->
```json
{"status":"active"}
```
<!-- intent-activation:end -->

The same batch makes the declarative architecture constraint executable:

<!-- constraint-check-patch:start -->
```json
{"check":"cargo test --test invoice_issued domain_does_not_import_adapter_modules -- --exact"}
```
<!-- constraint-check-patch:end -->

The initial runner is empty so bootstrap cannot launch a process. Stage its
complete public configuration in this first batch; this post-state becomes the
effective runner for the red/green work and is sealed by reconcile:

<!-- runner-config:start -->
```json
{"code":{"globs":["Cargo.toml","Cargo.lock","src/**/*.rs"]},"tests":{"globs":["tests/**/*.rs"]},"test":{"cmd":"cargo test {filter}"},"policy":{"tdd":"strict"},"agents":{"hosts":[]}}
```
<!-- runner-config:end -->

```console
telos change open "rebuild INT-0017" --json
printf '%s\n' '{"status":"active"}' | telos edit intent INT-0017 --change CHG-0001 --json
printf '%s\n' '{"check":"cargo test --test invoice_issued domain_does_not_import_adapter_modules -- --exact"}' | telos edit constraint CON-0003 --change CHG-0001 --json
printf '%s\n' '{"code":{"globs":["Cargo.toml","Cargo.lock","src/**/*.rs"]},"tests":{"globs":["tests/**/*.rs"]},"test":{"cmd":"cargo test {filter}"},"policy":{"tdd":"strict"},"agents":{"hosts":[]}}' | telos config --change CHG-0001 --json
telos change diff CHG-0001 --json
telos change approve CHG-0001 --json
telos rebuild status --json
mkdir -p src tests
```

The staged runner makes progress measurable without a proof target or process
invocation yet, so this first status is `0/2`.

The external implementer creates every runner input only after approval. Cargo
and source inputs are in `[code].globs`; the first batch binds and seals all
three. The exact `syn` dependency makes the architecture proof portable.

<!-- application-manifest:start -->
```sh
cat > Cargo.toml <<'TELOS_EOF'
[package]
name = "billing-rebuild"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"

[dev-dependencies]
syn = { version = "=3.0.3", features = ["full", "visit"] }
TELOS_EOF
```
<!-- application-manifest:end -->

The deliberately incomplete source gives the first unchanged proof test its
real red witness:

<!-- red-source:start -->
```sh
cat > src/lib.rs <<'TELOS_EOF'
// The scenario test must fail before the first implementation exists.
TELOS_EOF
```
<!-- red-source:end -->

The architecture proof shares the covered `SCN-0091` test target. It parses
Rust syntax, so comments, strings, and `adapters_v2` are harmless while real
`adapters` imports are rejected.

<!-- invoice-issued-test:start -->
```sh
cat > tests/invoice_issued.rs <<'TELOS_EOF'
use std::fs;
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
TELOS_EOF
```
<!-- invoice-issued-test:end -->

```console
telos test SCN-0091 --json
```

Now write only the first scenario's implementation. Its inert comment/string
and real `adapters_v2` reference demonstrate the syntax check's negative
space.

<!-- first-implementation:start -->
```sh
cat > src/lib.rs <<'TELOS_EOF'
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
TELOS_EOF
```
<!-- first-implementation:end -->

The red test bytes stay unchanged. Its green run creates the canonical
`proves` binding; the explicit binds cover every Cargo/source input:

```console
telos bind Cargo.lock INT-0017 --json
telos bind Cargo.toml INT-0017 --json
telos bind src/lib.rs INT-0017 --json
telos test SCN-0091 --json
telos change reconcile CHG-0001 --json
telos rebuild status --json
```

## Batch 2: activate settlement

```console
telos change open "rebuild INT-0042" --json
printf '%s\n' '{"status":"active"}' | telos edit intent INT-0042 --change CHG-0002 --json
telos change diff CHG-0002 --json
telos change approve CHG-0002 --json
```

<!-- payment-received-test:start -->
```sh
cat > tests/payment_received.rs <<'TELOS_EOF'
use billing_rebuild::{Invoice, InvoiceState};

#[test]
fn scn_0107_full_payment_settles_invoice() {
    let mut invoice = Invoice::issued_to("ACME", 12_000);
    invoice.receive_payment(12_000);
    assert_eq!(invoice.state(), InvoiceState::Settled);
}
TELOS_EOF
```
<!-- payment-received-test:end -->

```console
telos test SCN-0107 --json
```

The behavioral test can turn green while the architecture constraint remains
capable of rejecting a compiling violation. Both whitespace-separated and
grouped adapter imports are deliberate negative evidence:

<!-- constraint-violating-implementation:start -->
```sh
cat > src/lib.rs <<'TELOS_EOF'
pub mod adapters {
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
TELOS_EOF
```
<!-- constraint-violating-implementation:end -->

```console
telos bind src/lib.rs INT-0042 --json
telos test SCN-0107 --json
telos change reconcile CHG-0002 --json
```

That reconcile must return exactly `TELOS_CONSTRAINT_FAILED`. Repair only the
implementation, leaving the green scenario test untouched:

<!-- final-implementation:start -->
```sh
cat > src/lib.rs <<'TELOS_EOF'
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
}
TELOS_EOF
```
<!-- final-implementation:end -->

```console
telos change reconcile CHG-0002 --json
telos rebuild status --json
telos check --sealed --json
telos view --port 3000
telos view --export site
```

Progress is now `2/2`, both changes are closed, every constraint passes, and
the complete Cargo/source/test proof surface is sealed. The repository
acceptance test `cargo test -p telos --test rebuild_demo` consumes every
payload and heredoc above, performs the sequence twice in fresh git
repositories through the public CLI, and compares the observable plan,
red/green runs, progress, and final status.
