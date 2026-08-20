# Billing reconstruction demo

This directory is intentionally a spec-only Telos project. It ships no
`Cargo.toml`, application source, test, `telos.lock`, generated site, build
artifact, or hidden solution. Telos produces the ordered, bounded work plan
and verifies the result; an external `telos-implementer` agent writes the
application. The `telos` CLI itself makes no LLM call and does not generate
application code.

Inspect the prerequisite-first plan and its context packs from this directory:

```console
telos rebuild plan --json
telos rebuild status --json
```

Before implementation, generate the architecture check outside the configured
scenario-test glob. It recursively inspects Rust source without `grep` or a
platform-specific shell command:

```console
mkdir -p architecture
```

<!-- architecture-check:start -->
```sh
cat > architecture/hexagonal.rs <<'TELOS_EOF'
use std::fs;
use std::path::{Path, PathBuf};

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

#[test]
fn domain_does_not_import_adapter_modules() {
    let mut files = Vec::new();
    rust_files(Path::new("src"), &mut files);
    files.sort();
    for path in files {
        let source = fs::read_to_string(&path).expect("read domain source");
        for forbidden in [
            "use crate::adapters",
            "use self::adapters",
            "use super::adapters",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} imports the adapters layer through `{forbidden}`",
                path.display()
            );
        }
    }
}
TELOS_EOF
```
<!-- architecture-check:end -->

Create `Cargo.toml` with these exact bootstrap bytes. The inactive future
application target lets Cargo run the architecture check before application
source exists:

<!-- bootstrap-manifest:start -->
```sh
cat > Cargo.toml <<'TELOS_EOF'
[package]
name = "billing-rebuild"
version = "0.1.0"
edition = "2024"

[features]
application = []

[[bin]]
name = "application"
path = "src/main.rs"
required-features = ["application"]

[[test]]
name = "architecture"
path = "architecture/hexagonal.rs"
TELOS_EOF
```
<!-- bootstrap-manifest:end -->

Now perform the one-time bootstrap seal. At this point there is still no
application source, scenario test, or binding; the architecture constraint is
the only Cargo test target:

```console
telos change reconcile --full --json
telos status --json
```

The external implementer then handles each planned intent in order using the
ordinary protocol. It opens a batch and pipes `{}` to the public patch API.
That empty patch makes `edit intent` stage the complete existing post-state,
byte-identical to the base; `change diff` displays the complete equal
`before`/`after` bytes and a non-empty digest before approval:

```console
telos change open "rebuild INT-0017" --json
printf '%s\n' '{}' | telos edit intent INT-0017 --change CHG-0001 --json
telos change diff CHG-0001 --json
telos change approve CHG-0001 --json
```

Before writing the first scenario test, replace the bootstrap manifest with
the application manifest used for both batches:

<!-- application-manifest:start -->
```sh
cat > Cargo.toml <<'TELOS_EOF'
[package]
name = "billing-rebuild"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"

[[test]]
name = "architecture"
path = "architecture/hexagonal.rs"
TELOS_EOF
```
<!-- application-manifest:end -->

It writes the smallest discoverable behavioral test containing the literal
`scn_0091` token, records the real failure, writes only the needed application
code, binds that code, reruns the unchanged test to green, and reconciles:

```console
telos test SCN-0091 --json
telos bind src/lib.rs INT-0017 --json
telos test SCN-0091 --json
telos change reconcile CHG-0001 --json
telos rebuild status --json
```

Repeat the same batch for `INT-0042` / `SCN-0107`. Progress is measured by
executing the current proof targets and advances from `0/2` to `1/2`, then
`2/2`; bindings alone never count as green. Every reconcile also runs
`cargo test --test architecture`, so a domain import from `adapters` fails as
`TELOS_CONSTRAINT_FAILED` even when the behavioral scenario itself is green.

Verify and inspect the reconstructed project with the public commands:

```console
telos check --sealed --json
telos rebuild status --json
telos view --port 3000
telos view --export site
```

The repository acceptance test `cargo test -p telos --test rebuild_demo`
performs this complete sequence twice in fresh git repositories and compares
the observable plan, red/green runs, and progress results.
