# Audit fix — export staging identity

## Root cause

The static exporter reserved a predictable PID-plus-sequence staging name but
did not retain either the destination-parent handle, the staging-directory
handle, or the identity of the reserved directory entry. Every page write
reopened `staging.join(relative)`, publication reopened the staging pathname,
and error cleanup called `remove_dir_all` on that pathname.

Consequently, a deterministic actor could rename the genuine staging
directory and install a replacement directory or symlink at the same name.
The exporter then wrote through or published the replacement, and failure
cleanup could recursively remove replacement-owner data.

## TDD evidence

RED tests were added before production changes:

- `rtk proxy cargo test -p telos substituted_staging -- --nocapture`
  failed 2/2: a hostile replacement was published, and collision cleanup
  removed the replacement entry.
- `rtk proxy cargo test -p telos staging_symlink -- --nocapture`
  failed 1/1: writes followed a substituted staging symlink and the export
  returned success.

After the minimal implementation:

- `rtk cargo test -p telos view::export -- --nocapture` — 6 passed.
- `rtk cargo test -p telos --test view_export -- --nocapture` — 7 passed.

## Fix

- Staging names now contain 128 bits from the OS CSPRNG and are still reserved
  exclusively with `create_dir`.
- The destination parent and staging directory remain open as `cap_std::fs::Dir`
  capabilities for the transaction lifetime.
- Page parents are opened component-by-component without following symlinks;
  final files use relative `create_new` plus no-follow opens.
- The reserved staging entry records its cross-platform `(device, inode)`
  identity. The entry is reopened no-follow and compared with both the stored
  identity and held staging handle immediately before publication.
- Linux publication uses parent-relative `renameat2(RENAME_NOREPLACE)` and
  Darwin uses parent-relative `renameatx_np(RENAME_EXCL)`. Windows retains
  `MoveFileW`, whose create-only destination behavior is unchanged.
- Cleanup first repeats the identity check and then removes through the held
  staging handle. A missing, symlinked, or identity-divergent entry is never
  cleaned by pathname. No broad pathname-recursive deletion remains.

Rendered bytes, sorted response paths, the JSON envelope, existing-destination
errors, destination no-replacement behavior, and two-export determinism are
unchanged and covered by the existing integration suite.

## Security boundary

This closes deterministic and accidental staging substitution races. Portable
filesystem APIs do not provide one atomic operation that compares the source
entry identity and performs a no-replace directory rename. A same-UID
adversary able to continuously swap entries in the final check-to-rename
window is therefore not claimed to be defeated. The high-entropy name makes
pre-reservation guessing impractical, capability-relative writes prevent a
substituted path from redirecting page bytes, and cleanup refuses every
identity mismatch.

## Verification

- `rtk rustfmt --edition 2024 --check crates/telos/src/view/export.rs` — exit 0.
- `rtk cargo test -p telos view::export -- --nocapture` — 6 passed.
- `rtk cargo test -p telos --test view_export -- --nocapture` — 7 passed.
- `rtk cargo clippy -p telos --bin telos --tests -- -D warnings` — exit 0.

An additional `rtk cargo test -p telos` run reached two unrelated failures in
the concurrent, uncommitted Task 9 acceptance-loop work: `loop_merge` expected
`tests_run: 0` while reconcile returned `1`, and `loop_projection` unwrapped a
missing value at `acceptance_loops.rs:797`. Those files are explicitly outside
this fix and were not modified or staged here.

Only the exporter, its direct `getrandom` dependency/lock entry, unit tests in
the exporter module, and this report belong to the fix.
