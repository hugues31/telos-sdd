# Final review fix wave

Date: 2026-08-21

Branch: `worktree-feat-view-vue-spa`

Starting commit: `f9d486e`

Scope: the four final-review findings only

## Outcome

All four findings are fixed in one coherent wave. The live server now has a per-process, same-origin session boundary; live polling always re-synchronizes after startup, recovery, and server lifecycle changes; the graph filter exposes the complete canonical relation vocabulary on every model; and release builds and publication are pinned to one immutable source commit.

The final frontend and Rust verification suites pass. A real Chromium adversarial probe confirmed that a foreign-origin classic script cannot execute `/data.js`, even in the stronger same-site/different-port case where the browser sends the session cookie. Static `file://` export still makes zero network requests. Neither `Cargo.lock` nor `frontend/package-lock.json` changed.

## Finding 1 — live `/data.js` cross-origin execution

### Root cause

The live server treated CORS as the browser boundary for an executable classic script response. Classic `<script src>` inclusion does not depend on CORS, so any web origin could point at the loopback `/data.js` endpoint and run the serialized model in its own window. Binding to loopback did not prevent a remote page from driving the visitor's browser, and accepting arbitrary `Host` values also left a DNS-rebinding avenue.

### Implemented boundary

`LiveServer::bind` now creates one `LiveRequestBoundary` for the server process:

- `getrandom`, already present in the dependency graph, fills 32 random bytes (256 bits); they are hex encoded as the unpredictable session credential. No dependency or lockfile change was needed.
- The root shell response establishes `telos_view_session_<port>=<credential>; HttpOnly; SameSite=Strict; Path=/`. The port-scoped cookie name lets multiple concurrent loopback servers coexist in one browser cookie jar. `Secure` is intentionally omitted because the advertised development URL is plain HTTP loopback.
- Every request must carry the exact advertised authority, `Host: 127.0.0.1:<bound-port>`. A mismatch is rejected with `421 Misdirected Request` before routing. This is the DNS-rebinding boundary.
- `/data.js` and `/live.json` require the exact session cookie. Credential comparison is constant-time, and duplicate instances of the server's own cookie name are rejected. A missing or invalid session is `403 Forbidden`.
- When `Sec-Fetch-Site` is present on a sensitive request, only `same-origin` and `none` are accepted. `same-site`, `cross-site`, and unexpected values are `403 Forbidden`. Its absence remains supported for raw clients, which still must satisfy the exact `Host` and session checks.
- Sensitive success and denial responses use `Cache-Control: no-store` and `Cross-Origin-Resource-Policy: same-origin`. The shell also receives the same resource policy and `no-store` response treatment. Normal same-origin shell asset loading remains unaffected.
- Static export is unchanged: the credential and middleware exist only on the live HTTP server path, not in generated files.

### Threat boundary

The boundary protects the live model from a hostile web origin, including cross-site and same-site/different-port script inclusion, and rejects rebinding through an unadvertised `Host`. `SameSite=Strict` is not used as the sole defense: a same-site/different-port attacker can receive the cookie, but Fetch Metadata rejects it and CORP supplies an additional browser-enforced response boundary.

The design intentionally does not claim to isolate the server from another local process running as the same user. Such a process can make raw requests, obtain the root-set session, and access the user's local files by other means already. This non-goal and the exact raw-client handshake are documented in `docs/contracts.md`.

### Tests and browser evidence

Integration helpers now first request `/`, capture the cookie pair, and then access protected endpoints. Coverage includes no session, wrong session, duplicate cookie, exact-session success, absent Fetch Metadata raw clients, all rejected Fetch Metadata values, invalid `Host`, response headers, and distinct cookies for concurrent servers.

The final real Chromium probe used an attacker page at `http://127.0.0.1:<foreign-port>` and inserted a classic `<script src="http://127.0.0.1:<target-port>/data.js">`. This deliberately exercises the stronger same-site case rather than only a cross-site hostname:

```json
{
  "normal_live": {
    "status": 200,
    "corp": "same-origin",
    "generation_before": 0,
    "generation_after": 1
  },
  "graph": {
    "option_count": 9,
    "absent_filter": "refines"
  },
  "adversarial_same_site_script": {
    "cookie_sent": true,
    "sec_fetch_site": "same-site",
    "status": 403,
    "corp": "same-origin",
    "script_event": "error",
    "executed": false
  },
  "file_export": {
    "mode": "export",
    "option_count": 9,
    "external_requests": 0
  }
}
```

The normal live portion also verified shell boot, authenticated status/data traffic, a source-file change, generation advancement, and visible hot replacement.

## Finding 2 — stale live startup and recovery

### Root cause

The poller used the first successful status only to establish a generation baseline. If `/data.js` had changed between the shell's initial script load and that first status response, no reload occurred. Generation was also treated as monotonically increasing, so a restarted server advertising a lower generation was ignored. Finally, an error did not create a durable synchronization obligation for a later status with the same number.

### Implemented synchronization model

The controller now tracks a `needsSynchronization` obligation independently from the last observed generation:

- Every `start()` requires synchronization, including restarting the same controller instance.
- The first valid status always requests a data reload, even if shared HMR state already records the same generation.
- A status/data network error, malformed status, rejected payload, or load failure re-arms synchronization. The next valid status reloads even when its generation is unchanged.
- Any sequential generation inequality reloads. A decrease is treated as a new server lifecycle, resets the observed generation, and replaces the snapshot.
- If a controller adopts an older in-flight HMR load, its reload loop immediately drains the newer outstanding generation rather than waiting for another poll.

The existing guarantees remain intact: export mode never polls; polling stays sequential; `stop()` and HMR share and invalidate obligations correctly; obsolete in-flight results cannot overwrite newer visible data; and the payload guard still runs before an atomic snapshot swap.

### Regression coverage

Tests cover startup interleaving, a first equal shared generation, connection failure followed by the same generation, `10 -> 9` server restart, restart of the same controller, growing generation, retried failed generation, shared in-flight HMR adoption, stale result suppression, stop, malformed status/payload, and export zero-network behavior.

## Finding 3 — canonical graph relation filters

One typed canonical ordered constant now defines the graph relation vocabulary:

1. `refines`
2. `requires`
3. `excludes`
4. `constrains`
5. `verifies`
6. `uses`
7. `implements`
8. `proves`

`GraphRelation` is derived from that constant, the live payload guard consumes it, and the graph option helper returns `All` followed by those eight relations for every model. `GraphPage` always renders the selector, including empty/no-edge models, and no longer resets a selected relation merely because no current edge has it.

Pure tests assert the exact sparse and empty option lists. Mounted graph behavior verifies that selecting an absent canonical relation is valid, yields zero matches/dims all graph elements, and uses the existing filter update path without relayout. The Chromium probe observed nine options and successfully selected absent `refines`.

## Finding 4 — immutable release source

The release graph now resolves source identity exactly once:

```text
frontend checkout (tag input or immutable push SHA)
  -> output source_sha + frontend-dist artifact
  -> build[6 targets] checkout source_sha and package artifacts

frontend + build
  -> publish checkout source_sha
  -> refetch requested tag and peel ^{commit}
  -> require tag commit == source_sha
  -> checksums and GitHub Release publication
```

For a tag-push event, frontend checks out immutable `github.sha`; for `workflow_dispatch`, it resolves the requested tag once. All six build jobs consume `needs.frontend.outputs.source_sha`, and `SOURCE_DATE_EPOCH` comes from that pinned checkout. Publish waits on both frontend and build, checks out the pinned SHA, force-fetches only the requested tag, resolves its commit (including annotated tags), and fails before checksums or publication if it moved.

Artifact names, download pattern, matrix, workspace version guard, packaging commands, checksum layout, and create-or-upload release behavior are preserved.

A YAML semantic validator checked the dependency graph, source output propagation, all checkout expressions, tag input/push fallbacks, publication ordering, and tag equality guard. A local Git simulation confirmed the guard accepts a stable annotated/lightweight ref resolution and exits nonzero after the same tag is force-moved:

```text
Tag v0.7.0 moved from a179858... to c23868...
exit status: 1
```

## TDD evidence

Tests were added or strengthened before implementation for each behavior change.

| Finding | RED evidence | GREEN evidence |
| --- | --- | --- |
| Live boundary | The new protected-data integration test received `HTTP/1.1 200 OK` where `403` was required. The concurrent-server test also exposed the same predictable cookie name on both ports. | Focused server integration tests pass with session establishment, 421 Host rejection, 403 origin/session rejection, and concurrent port-scoped cookies. |
| Live synchronization | Four new cases reported zero loads: startup first status, first equal generation, post-network same generation, and lower generation. Follow-up regressions exposed missed new-generation draining after HMR adoption and failure to re-arm a restarted controller. | All synchronization cases pass, including immediate drain of shared in-flight work and `10 -> 9` lifecycle reset. |
| Graph options | The new pure option test failed because `./relations` did not exist. | Exact empty and sparse canonical-option tests pass; mounted absent-filter behavior passes. |
| Release identity | Semantic assertions failed because build jobs independently checked out the mutable tag and publish depended only on build. | The graph/output/expression validator passes, and the moved-tag simulation fails closed. |

Representative RED/GREEN commands were:

```text
rtk cargo test -p telos --test view_server live_data_requires_the_exact_loopback_session_and_browser_origin -- --nocapture
rtk npm test -- --run src/data/live.test.ts
rtk npm test -- --run src/graph/relations.test.ts src/graph/elements.test.ts
rtk python3 /tmp/validate_telos_release.py
```

## Final verification

The full frontend and Rust commands below were run after the final source changes; the Rust suite was run after rebuilding `frontend/dist`. Focused tests listed below were also run during the TDD loop and are subsumed by the final full suites.

| Command | Result |
| --- | --- |
| `rtk npm test -- --run` | 7 files, 95 tests passed |
| `rtk npm run typecheck` | passed |
| `rtk npm run build` | passed; final Rust-embedded `frontend/dist` produced |
| second build plus `rtk diff -r <reference-dist> dist` | passed, byte-for-byte deterministic |
| real Chromium live/adversarial/file probe | normal live reload passed; hostile classic script 403/error/not executed; file export zero network |
| static export file manifest check | exact six expected files produced |
| `rtk cargo test --workspace --locked` | 1,101 tests passed across 34 suites |
| `rtk cargo fmt --all --check` | passed |
| `rtk cargo clippy --workspace --all-targets --locked -- -D warnings` | passed |
| focused telos bin tests | 71 passed |
| `view_export` integration | 7 passed |
| `view_server` integration | 10 passed |
| projection acceptance loop | 1 passed |
| published-contract filters | 15 passed |
| release YAML semantic validator | passed for push-tag and `workflow_dispatch` paths |
| local moved-tag simulation | failed closed with exit status 1, as required |
| `rtk git diff --check` | passed |
| lockfile diff | empty for `Cargo.lock` and `frontend/package-lock.json` |

The Vite build continues to emit the already-known warning that `data.js` is a classic script and the existing bundle-size advisory. The classic script is required for offline `file://` export; the new live-only HTTP boundary is what makes it safe in server mode. These warnings are unchanged by this wave.

`actionlint` was not installed in the environment. Workflow validation therefore used YAML parsing, a purpose-built semantic graph/expression validator, and an actual Git moved-tag simulation rather than claiming an unavailable check.

## Files changed

- `.github/workflows/release.yml`
- `crates/telos/src/view/server.rs`
- `crates/telos/tests/acceptance_loops.rs`
- `crates/telos/tests/view_server.rs`
- `docs/contracts.md`
- `frontend/src/data/live.ts`
- `frontend/src/data/live.test.ts`
- `frontend/src/data/types.ts`
- `frontend/src/graph/relations.ts`
- `frontend/src/graph/relations.test.ts`
- `frontend/src/graph/elements.test.ts`
- `frontend/src/pages/GraphPage.vue`
- `.superpowers/sdd/je-veux-refactorer-l-aper-u-kind-planet/final-fix-report.md`

`frontend/dist` was rebuilt and is present for Rust embedding, but remains intentionally ignored. No package or Cargo dependency metadata changed.

## Self-review and concerns

- The server boundary is layered: exact Host, strong per-process session, Fetch Metadata, CORP, and no-store. No predictable token, URL credential, CORS assumption, or new dependency remains.
- Cookie scoping was reviewed against simultaneous loopback servers. Because cookies cannot be scoped by port, the port is part of the cookie name and each server selects only its own credential.
- Status codes are deliberate: 421 communicates an authority mismatch; 403 communicates a valid authority with an invalid browser/session boundary. Sensitive refusals do not reveal the model.
- The live controller's synchronization obligation is separate from generation comparison, which closes both the startup race and all recovery paths. Its immediate drain loop preserves sequential requests while preventing an adopted stale load from delaying the latest generation.
- The graph vocabulary has one source of truth; options and payload validation cannot drift independently.
- Release publication cannot proceed if the requested tag changes after the upstream resolution. Every build's timestamp and checkout derive from the same immutable commit.
- No unrelated ledger minor was changed.
- Remaining non-blocking environmental concern: `actionlint` was unavailable; the checked semantic validator and Git simulation cover the requested workflow properties, but hosted Actions remains the ultimate execution environment.
