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

## Round 2 — source-bound publication and owner-safe failure

The first fix was rejected after four findings, all confirmed against the
implementation and dependency sources:

1. Linux `renameat2` and Darwin `renameatx_np` bind the parent directory but
   still resolve the source object from a pathname. A replacement after the
   identity check was therefore published.
2. `create_dir` returns no directory handle. Reopening the name and then
   recording its identity could adopt a replacement installed between the two
   calls.
3. `cap_std` deliberately opens Windows directories without
   `FILE_SHARE_DELETE`; Microsoft documents that rename/delete access requires
   that sharing flag on every extant handle. `MoveFileW` therefore conflicted
   with the staging handle retained by the first fix.
4. `cap_std`'s Windows `remove_open_dir_all` obtains the handle's path, closes
   the handle, and removes by pathname. Its source explicitly documents this
   unavoidable race.

### Additional RED evidence

- `rtk proxy cargo test -p telos between_creation_and_open -- --nocapture`
  returned `Ok` and published the injected reservation replacement.
- `rtk proxy cargo test -p telos after_identity_check -- --nocapture` returned
  `Ok` and published the replacement installed after the last identity check.
- `rtk proxy cargo test -p telos explicitly_incomplete -- --nocapture` failed
  because a forced write error removed the staging directory and left no
  explicit incomplete destination.

### Replacement architecture

Round 2 removes staging-directory publication entirely:

- The final destination is reserved directly with create-only `create_dir`.
- It is opened no-follow, must still be empty, and remains held for the whole
  transaction. A symlink or non-empty substitution is refused before page
  writes.
- `index.html` is created first through the held handle with a small explicit
  `Telos export incomplete` page. All remaining files use relative,
  component-by-component no-follow traversal and `create_new`.
- Before completion, the exporter enumerates the held tree and verifies the
  exact expected paths and bytes. It then rewrites the already-held
  `index.html` file with the rendered dashboard and verifies the final tree.
- There is no source rename, pathname publication, recursive cleanup, or file
  deletion. A failed transaction remains at the create-only destination with
  an incomplete index (or partial owned output) and every retry refuses the
  existing destination. No foreign entry is ever published or cleaned up by
  the exporter.
- The Linux/Darwin `libc`, Windows `windows-sys`, and random staging-name
  dependencies are no longer needed and were removed.

Successful output still has the exact six sorted paths and bytes from Task 4;
the incomplete index is overwritten in place and adds no success-only file to
the envelope or tree. Existing destination and symlink collision behavior is
unchanged.

### Why Unix cannot retain atomic directory publication here

`mkdir`/`mkdirat` return only a status code, not the created directory object.
`renameat`/`renameat2` and Darwin's `renameatx_np` accept parent descriptors but
still require a source pathname. POSIX prohibits hard-linking directories, so
the regular-file technique of linking a held inode into a create-only final
name is unavailable. Without privileged mount operations, the portable APIs in
scope provide neither create-and-return-handle nor rename-directory-by-handle.

The residual same-UID boundary is explicit: an empty directory replacement in
the `mkdir`-to-open interval is indistinguishable from the just-created empty
directory. Such an empty replacement contains no foreign bytes to publish;
all subsequent writes are create-only, exact contents are checked, and the
exporter never removes it. A non-empty or symlink replacement is rejected by
the deterministic tests. Continuous same-UID mutation after the final check is
likewise outside any filesystem protocol available here.

### Round 2 verification

- `rtk cargo test -p telos view::export -- --nocapture` — 7 passed.
- `rtk cargo test -p telos --test view_export -- --nocapture` — 7 passed.
- `rtk rustfmt --edition 2024 --check crates/telos/src/view/export.rs` — exit 0.
- `rtk cargo clippy -p telos --bin telos --tests -- -D warnings` — exit 0.
- `rtk git diff --check` — exit 0.
- Windows and Darwin target checks were attempted but both sysroots are absent
  from the image (`can't find crate for core/std`). The exporter now contains
  no OS-specific code or FFI; it uses the same `cap_std` calls on all targets.

## Round 3 — restore atomic publication

Round 2 was rejected because it fixed pathname ownership by abandoning the
Task 4 atomic-publication contract. It created the final destination before
writing, deliberately left partial output after errors, and completed by
truncating and rewriting `index.html` in place. That rewrite could itself fail
after truncation or after writing apparently complete dashboard bytes. The
create-directory-to-open gap also still adopted an empty replacement because
the entry identity was first recorded only after the test hook and reopen.

### Round 3 RED evidence

The public contract was restored in tests before production code changed:

- `rtk proxy cargo test -p telos a_staging_write_failure_leaves_no_destination_or_staging_directory -- --nocapture`
  failed because `site` remained after the injected page write error.
- `rtk proxy cargo test -p telos a_finalization_failure_leaves_no_destination_or_staging_directory -- --nocapture`
  failed because `site` remained after the injected finalization error.
- `rtk proxy cargo test -p telos a_staging_entry_substituted_between_creation_and_open_is_not_adopted -- --nocapture`
  returned `Ok` and published the empty directory installed by the hook.
- `rtk proxy cargo test -p telos a_staging_entry_substituted_after_identity_check_is_not_published -- --nocapture`
  left the hook's replacement at the final destination path.

After the implementation, each exact command passed individually. The full
unit module then passed 8/8 and the real-binary `view_export` suite passed 7/7.

### Restored architecture

- All six rendered pages are still completed in memory first.
- A sibling staging directory is reserved create-only with a 128-bit CSPRNG
  suffix. Immediately after `create_dir` returns, its no-follow `(device,
  inode)` identity is recorded **before** the create-to-open hook. The no-follow
  opened directory must match that recorded identity before any page write.
- Every child directory is traversed no-follow. Every page is opened
  `create_new`, written through the held staging capability, and `sync_all` is
  called. The exact sorted paths and every byte are re-read and compared with
  the in-memory render before finalization.
- A finalization hook can fail after the complete validation but before any
  publication. Ordinary write/finalization failures delete only the held,
  identity-matched staging object and leave no final destination.
- Publication checks destination absence and staging identity, runs the exact
  verify-to-publish hook, and repeats both checks before one no-replace rename.
  Linux retains `renameat2(RENAME_NOREPLACE)` and Darwin retains
  `renameatx_np(RENAME_EXCL)`. Success exposes the complete tree in one atomic
  namespace operation; collision preserves the existing destination.
- A replacement or symlink installed at either deterministic hook is never
  written through, published, or cleaned up. When identity diverges, cleanup
  deliberately leaves both the replacement and any displaced genuine staging
  tree alone.

There is no incomplete marker and no in-place completion rewrite in Round 3.
The success envelope, sorted file list, bytes, relative links, deterministic
two-export behavior, and exact destination-collision error are unchanged.

### Platform-bound ownership proof

On Linux, `mkdirat(dirfd, pathname, mode)` returns only success/failure, while
`renameat2(olddirfd, oldpath, newdirfd, newpath, flags)` still identifies its
source by `oldpath`; it has no `AT_EMPTY_PATH` form. POSIX also prohibits
ordinary hard links to directories. Darwin's exclusive rename likewise takes
a source parent descriptor plus pathname. Consequently neither platform can
both create a directory and later publish that directory by retained object
handle using the unprivileged filesystem APIs in scope. The Linux signatures
and `RENAME_NOREPLACE` semantics are documented by
<https://man7.org/linux/man-pages/man2/renameat2.2.html>; Apple's volume
contract identifies `RENAME_EXCL` as exclusive rename support at
<https://developer.apple.com/documentation/foundation/urlresourcekey/volumesupportsexclusiverenamingkey>.

Windows now uses a stronger source-bound sequence. cap-std's ordinary
directory capability remains open while writing, preventing rename because it
does not share delete access. Immediately before publication it is closed, a
no-follow `DELETE | FILE_SHARE_DELETE` directory handle is opened and checked
against the recorded identity, and `SetFileInformationByHandle(FileRenameInfo)`
renames that object relative to the held parent handle with
`ReplaceIfExists = false`. Microsoft documents that false means the operation
fails when the destination exists:
<https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info>.
Cleanup recursively protects entries with no-follow handles and finally marks
the exact staging directory handle with `FileDispositionInfo`; it does not use
cap-std's path-recovered Windows `remove_open_dir_all` and does not use
`MoveFileW`.

### Threat boundary and its cost

The project design §5 explicitly protects against negligence and ordinary
concurrency, not a same-UID adversary actively observing and forging entries.
That exclusion is material on Unix/Darwin: such an adversary could enumerate
the high-entropy name and replace it either between `create_dir` and the
immediate identity capture, or after the last identity check and before the
path-based rename syscall. In the first micro-window Telos could authenticate
the forged empty directory; in the second it could rename forged content.
There is no claim that the protocol prevents those adversarial outcomes.

Within the stated model, exclusive high-entropy reservation removes guessing,
capture-before-hook closes the deterministic create-to-open race, the second
check closes the deterministic verify-to-publish race, capability-relative
writes prevent symlink redirection, and the kernel no-replace operation closes
the destination collision. Windows additionally binds publication and root
cleanup to the authenticated source object.

### Round 3 verification

- `rtk cargo check -p telos --tests` — exit 0.
- `rtk cargo test -p telos view::export -- --nocapture` — 8 passed.
- `rtk cargo test -p telos --test view_export -- --nocapture` — 7 passed.
- `rtk rustfmt --edition 2024 --check crates/telos/src/view/export.rs` — exit 0.
- `rtk cargo clippy -p telos --bin telos --tests -- -D warnings` — exit 0.
- `rtk git diff --check` — exit 0.
- Windows and Darwin target checks were attempted; both stop before compiling
  Telos because their `core/std` sysroots are absent from the image. The
  Windows cfg is nevertheless parsed by rustfmt, and its API names/signatures
  were checked against the installed `windows-sys 0.61.2` bindings.
