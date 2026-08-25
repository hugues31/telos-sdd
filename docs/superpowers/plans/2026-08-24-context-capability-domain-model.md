# Context and Capability Domain Model — Implementation Plan

> **Execution rule:** implement every behavior test-first and keep the change deliberately breaking. Telos 0.9.0 does not read or generate the legacy flat layout or lock format.

## Goal

Make Telos scale to complex codebases by making strategic domain boundaries executable rather than advisory. Every specification element has an explicit owner, cross-context dependencies are declared and validated, vocabulary is context-qualified, and source ownership can be checked without trusting an LLM.

## Canonical layout

```text
telos/
  contexts/<context>/context.tel
  contexts/<context>/notions/<Notion>.tel
  contexts/<context>/constraints/<CON>.tel
  contexts/<context>/bindings.tel
  contexts/<context>/capabilities/<capability>/capability.tel
  contexts/<context>/capabilities/<capability>/notions/<Notion>.tel
  contexts/<context>/capabilities/<capability>/intents/<INT>.tel
  contexts/<context>/capabilities/<capability>/constraints/<CON>.tel
  constraints/<CON>.tel
  context-map.tel
  changes/
  telos.toml
  telos.lock
```

The old `telos/notions`, `telos/intents`, `telos/constraints` and project-level `telos/bindings.tel` layout is rejected with `TELOS_LAYOUT_VIOLATION`.

## Domain identities and ownership

- `ContextId`: validated lower-kebab identifier.
- `CapabilityId`: validated lower-kebab identifier.
- `CapabilityRef { context, capability }`.
- `NotionRef { context, notion }`.
- `Owner { context, capability: Option<CapabilityId> }`.
- Intent, scenario and constraint numeric IDs remain globally unique.
- A notion's identity is `(ContextId, NotionName)`. The same notion name may exist in different contexts.
- A capability is an ownership/location boundary, not part of notion identity.
- Every intent belongs to exactly one capability.
- A constraint's scope is inferred from its canonical path: project, context, or capability.

## DSL

```telos
context pet core "Pet" {
  def "Owns the rules and state of a virtual pet"
}

capability pet/care "Care" {
  def "Actions through which an owner cares for a pet"
}

notion pet/Pet entity {
  def "The virtual pet governed by the pet context"
}

notion pet/care/FeedPet event {
  def "A feeding accepted by the care capability"
}

intent INT-0002 in pet/care "Feed a pet" {
  ...
}

constraint CON-0001 in context pet quality "Vitals stay within bounds" {
  ...
}

context-map {
  dependency terminal on pet {
    map pet/Pet -> terminal/PetView
  }
}
```

Ordinary references resolve within the owning context. Context-map mappings always use qualified notion references. `refines` and `excludes` stay inside a context. A cross-context `requires` is valid only in the direction of a declared dependency.

## Deterministic rules

The semantic engine, never an agent prompt, enforces:

1. Every context, capability and owner exists.
2. Every file is at the canonical path implied by its declared identity and owner.
3. Notion names are unique within a context but may repeat across contexts.
4. Direct cross-context notion references are forbidden outside `context-map.tel`.
5. Context dependencies are directed and acyclic.
6. Every mapping endpoint exists and follows its dependency direction.
7. A supplier change impacts direct and transitive consumers.
8. A production source file may bind to intents from only one context.
9. A test file may prove scenarios from multiple contexts.
10. Ownership cannot be changed by an ordinary edit; it requires a tracked move.

Use `TELOS_LAYOUT_VIOLATION` for layout/owner mismatches and `TELOS_CONTEXT_BOUNDARY_VIOLATION` for semantic boundary failures.

## CLI contract

- Add `context` and `capability` entity kinds.
- Rename the work-pack command from `telos context` to `telos pack`; do not keep an alias.
- Add typed selectors: `CTX:pet`, `CAP:pet/care`, `NOT:pet/Pet`, plus existing numeric IDs.
- Add `--context` and `--capability` filters to list/query commands.
- Add `telos move <selector> --to <owner> --change <CHG>`.
- Add `telos map` and `telos map --change` for context-map changes.
- Include `owner` in machine-readable JSON.

Moves claim both the old and new path, preserve the entity identity, and invalidate approval. `adopt` pairs an untracked and a missing file with the same identity into a move; a lone misplaced file fails.

## Bindings, packs, graph and lock

- Store bindings per context at `telos/contexts/<context>/bindings.tel`.
- Production files cannot implement multiple contexts; tests can cover multiple contexts.
- A `telos pack` contains the selected owner, local/shared notions, project/context/capability constraints, local bindings and proofs, plus only the necessary context-map mappings. It must not expose supplier internals.
- Add `Context` and `Capability` graph nodes plus `BelongsTo`, `DependsOn` and `MapsTo` edges.
- Bump the lock format to version 2 and reject version 1 with an actionable error.

## Interface and agents

- Add a Contexts page showing contexts, capabilities, dependencies, mappings and health.
- Group/filter glossary and intents by context and capability.
- Render the new graph nodes and relations.
- Update all agent skills/prompts/replay fixtures to use `telos pack` and qualified ownership.
- Agents may challenge language and suggest boundaries, but engine rules remain the source of truth.

## Internal migration

Migrate the repository's billing example to context `billing`:

- capability `invoicing`: `InvoiceIssued`, `INT-0017`;
- capability `settlement`: `PaymentReceived`, `INT-0042`;
- shared notions: `Customer`, `Invoice`;
- `CON-0003` scoped to the billing context.

Remove all legacy flat spec files and regenerate `telos.lock` as version 2.

## Tamagotchi migration

Migrate `/home/hugues/Bureau/telos-tamagotchi` to:

```text
pet (core)
  lifecycle: INT-0001, INT-0004, INT-0008, INT-0009, INT-0010
  care:      INT-0002, INT-0003, INT-0005, INT-0006, INT-0007, INT-0011
  shared:    Pet, Owner, TimeTicks

terminal (supporting)
  portrait:  INT-0012
  notions:   Portrait, RenderRequested, PetView
```

Declare `terminal depends on pet` and map `pet/Pet -> terminal/PetView`. Scope `CON-0001` to `pet` and keep `CON-0002` project-wide. Remove the artificial `pytest.ini` and `tamagotchi/__init__.py` bindings and narrow code globs to meaningful source files.

## TDD execution sequence

### 1. Identities, entities and grammar

1. Add failing unit tests for identifier validation, qualified notion identity, owner parsing and all new DSL forms.
2. Implement the model types, AST nodes, parser and formatter.
3. Run focused parser/model tests, then `cargo test -p telos-core`.

### 2. Canonical recursive workspace

1. Add failing workspace tests for recursive discovery, canonical path derivation and explicit rejection of legacy/misplaced files.
2. Replace flat scanning and hard-coded paths with owner-derived canonical paths.
3. Verify focused workspace/change tests.

### 3. Semantic boundaries and graph

1. Add failing semantic tests for ownership, local uniqueness, dependency direction/cycles, mappings, cross-context references and transitive impact.
2. Implement validation and graph nodes/edges.
3. Verify semantic and graph suites.

### 4. Transactions, moves and CLI

1. Add failing transaction and CLI integration tests for typed selectors, moves, map changes, filters and the `pack` rename.
2. Implement move claims and approval invalidation; remove the old `context` command.
3. Verify CLI snapshots and integration tests.

### 5. Bindings, reconcile, drift and lock v2

1. Add failing tests for per-context bindings, single-context production ownership, multi-context tests, move adoption, drift and v1 lock rejection.
2. Implement the new binding index and lock serialization.
3. Verify reconcile/drift/lock suites.

### 6. Packs, queries and agent contracts

1. Add failing tests proving exact pack inclusion/exclusion and owner JSON.
2. Implement pack/query/rebuild changes and update agent-facing instructions.
3. Verify CLI, replay and agent contract tests.

### 7. SPA

1. Add failing component/store tests for context navigation, grouping, filters and graph relations.
2. Implement the Contexts page and update existing views.
3. Run frontend unit tests and type checking.

### 8. Repository migration and release metadata

1. Migrate all Telos examples/specifications to the billing context layout.
2. Update README/contracts/prompts/snapshots and bump crate/app metadata from 0.8.2 to 0.9.0.
3. Regenerate and verify lock/proof fixtures.

### 9. Tamagotchi migration

1. Migrate the external demo to `pet` and `terminal` using the projection above.
2. Run Telos validation/test/drift commands and the Python test suite.
3. Commit the demo migration separately if repository permissions allow it.

## Final verification

Run, from a clean diff review:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd frontend && npm test -- --run
cd frontend && npm run typecheck
```

Also execute Telos's end-to-end validation against this repository and the Tamagotchi repository. Report any external-repository write restriction explicitly; do not claim that migration complete without fresh evidence from both repositories.
