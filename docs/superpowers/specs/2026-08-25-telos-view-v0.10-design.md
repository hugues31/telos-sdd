# Telos View 0.10 Design

## Objective

Ship Telos 0.10.0 as a focused usability release for `telos view`. The release makes the shared navigation usable on phones, localizes the footer build date, adds graph node finding, makes intent details faithfully expose their canonical `.tel` behavior, clarifies glossary consumers, and aligns the interface with the blue/cyan logo.

## Scope

The release changes the embedded Vue application, its serialized view contract, the canonical emitter surface needed by that contract, fixtures and tests, and workspace release metadata. It does not change `.tel` syntax, model semantics, graph relations, live-reload transport, routes, or CLI behavior outside the reported version.

## Design Principles

- The Rust model and canonical emitter remain the source of truth for `.tel` meaning and formatting.
- Vue receives display-ready canonical fragments; it does not parse `.tel` or mirror the complete Rust AST.
- Search finds and focuses graph nodes without removing unrelated graph structure.
- Existing glossary cards remain the primary notion presentation.
- Blue is the brand color. Green is reserved for healthy or successful state.
- Every new control is keyboard accessible and remains usable at narrow widths.

## View Contract and Canonical Parity

`telos-core` will expose narrow canonical formatting functions for the two nested structures that the view needs to show independently:

- a complete `statement <template> { ... }` fragment for an intent statement;
- a complete `scenario SCN-NNNN "..." { ... }` fragment for a scenario.

These functions will reuse the existing emitter implementation, including indentation, expression formatting, quoting, and trailing newline rules. They must not introduce a second formatter.

The serialized view contract will add:

```text
StatementView {
  template: "ubiquitous" | "event-driven" | "state-driven" | "unwanted" | "optional"
  canonical: string
}

IntentView {
  ...existing fields
  statement: StatementView
}

ScenarioView {
  ...existing fields
  canonical: string
}
```

`IntentView.canonical` remains the complete intent file and `ScenarioView.canonical` remains a nested fragment, so the full-source fallback and focused scenario presentation are both authoritative. The TypeScript types, strict live-payload validator, development fixture, and Rust serialization tests will evolve atomically with this contract. A live payload that lacks or malforms the new fields is rejected and the client retains its last valid snapshot.

This approach is preferred over serializing the full model AST because it avoids coupling Vue to grammar internals. It is preferred over parsing `IntentView.canonical` in Vue because it cannot drift from the canonical emitter.

## Mobile Header

Desktop navigation remains inline. At viewport widths of 48rem and below, the header becomes one fixed-height row containing:

- the logo and project-health indicator;
- the theme toggle;
- a menu button with an accessible name and `aria-expanded` state.

The menu opens a full-width surface directly below the sticky header row. Links are stacked with a clear active state and touch-friendly targets. The menu closes when a destination is selected, the route changes, Escape is pressed, or a pointer interaction occurs outside the header. Opening the menu moves focus to the first navigation link; closing it with Escape returns focus to the menu button. The dropdown overlays page content, so `--header-height` continues to describe the sticky row and existing scroll margins remain correct.

## Localized Footer Date

The build date remains an ISO `YYYY-MM-DD` value in the payload. A pure frontend formatter validates the syntax and calendar date, constructs a local calendar date from numeric year/month/day components, and formats it with `Intl.DateTimeFormat` using the browser locale and a medium date style.

Constructing the date from local components avoids the previous-day error that can occur when a date-only ISO string is interpreted as UTC. If validation or formatting fails, the footer displays the original payload string. The version and `built` wording remain unchanged.

## Graph Finder

The graph heading gains a finder next to the relation filter. It behaves as an accessible combobox backed by all raw graph nodes, including descendants hidden by collapsed context or capability containers.

Matching is case-insensitive across node ID, display label, and kind. Results are ranked in this order:

1. exact ID or label;
2. ID or label prefix;
3. ID or label substring;
4. kind substring.

Ties retain snapshot order, and at most eight results are shown. Each result displays its kind, readable label, and stable ID. Arrow keys move through results, Enter selects the active result (or the first result when none is active), and Escape closes the result list without changing the graph. An unmatched non-empty query shows `No matching nodes`.

Selecting a result removes its context/capability ancestors from the collapsed set, waits for the visible graph to be rebuilt and laid out, sets the normal graph selection, centers the Cytoscape viewport on the node without hiding other nodes, and closes the result list. Re-selecting the same result issues a fresh focus request. If live reload removes the result before focus completes, the request safely becomes a no-op and any invalid selection is cleared through the existing selection normalization.

Search ranking and ancestor expansion remain pure helpers. Cytoscape owns only the final focus operation after it has applied the latest elements and layout.

## Intent Detail Parity and UX

The intent detail page will present information in this order:

1. back navigation, intent ID/title, status, and owning context or capability;
2. the telos statement as the lede;
3. an always-visible Statement card with the template label and syntax-highlighted canonical statement fragment;
4. intent-to-intent relations with directional labels such as `Requires` and `Required by`;
5. notions, constraints, and implementations;
6. scenario cards;
7. the complete canonical intent source in a collapsed reference section.

Each scenario card shows its ID/title, the complete syntax-highlighted canonical scenario fragment (therefore every `given`, `when`, and `then` line), and its proof state. Proof links remain unchanged. The page therefore exposes all behavior visible in a canonical intent such as `INT-0011` without requiring the reader to open the full-source disclosure, while still avoiding a frontend grammar implementation.

The existing graph-derived relation data remains authoritative. Dedicated Notions, Constraints, Implementations, and Scenarios sections continue to own those relation types; the Relations section focuses on `refines`, `requires`, and `excludes`, in both directions, with human directional labels instead of generic outgoing/incoming terminology.

## Glossary Consumers

Glossary notion cards, definitions, owner grouping, and canonical-source disclosures remain. The existing chip list under `Used by` becomes a reusable accessible table with these columns:

- Type: an Intent or Scenario kind pill;
- Reference: the stable `INT-NNNN` or `SCN-NNNN` identifier;
- Title: a link to the intent page or scenario anchor.

Headers sort Type, Reference, or Title in ascending/descending order. Default order is Type then Reference. Stable IDs remain visibly separate from titles; they are not relegated to hover text.

The glossary sidebar adds a `Used by type` filter with All, Intent, and Scenario options. Selecting a type retains only notions with at least one matching consumer and filters each displayed table to that type. This filter composes with the existing text and owner filters. The empty copy distinguishes a notion with no consumers from a filter that has no matching consumers.

Consumer rows continue to be derived from `uses` graph edges rather than duplicated in the payload. Pure helpers resolve row labels, filter by kind, and sort without mutating snapshot order.

## Brand Palette

The primary light-theme family will move from green to a blue derived from the logo's dominant `#0162fb`, with a darker blue for links and text contrast and a pale blue for selected or soft surfaces. The logo cyan `#03e1f0` will be available as a restrained accent for decorative emphasis, not normal text on light backgrounds.

The dark theme will use lighter blue for interactive text and focus states, a deep blue soft surface, and a brighter cyan accent. Backgrounds, borders, and neutral text shift slightly cooler so the palette feels intentional. Primary buttons, links, focus rings, active navigation, intent identity, and `.tel` keywords adopt the blue family. Green remains on coherent/healthy status and other success semantics. Entity-kind colors stay distinguishable from one another in both themes.

All changed text, focus, and interactive colors must retain accessible contrast against their actual surfaces. Color never becomes the only indication of status, kind, selection, or active navigation.

## Responsive Behavior

The graph finder and relation filter wrap into a single-column control area on narrow screens. The graph workspace continues to stack the selection panel below the canvas. Glossary tables are placed in horizontally scrollable containers when their columns cannot fit without truncating stable identifiers. Intent canonical fragments scroll horizontally inside their cards rather than widening the page.

Touch targets for the mobile menu, graph results, sort headers, and filters are at least 2.5rem high. Focus order follows visual order.

## Error and Empty States

- Invalid live snapshots preserve the last valid payload and surface the existing refresh error.
- Invalid build dates display the original string.
- Empty graph queries show no result popup; unmatched non-empty queries show `No matching nodes` and leave selection untouched.
- A stale graph focus request is ignored after live reload.
- Notions with no consumers say `No consumers recorded.`
- A selected consumer type with no rows is represented by the glossary-level empty state after all filters compose.
- Unknown intent IDs keep the existing not-found view and back link.

## Testing

Rust unit tests will prove:

- standalone canonical statement fragments for all five statement templates reuse canonical formatting;
- canonical scenario fragments preserve multiple given/then lines, literals, and compound expressions;
- `ViewSnapshot` includes the correct statement template/canonical fragment and scenario canonical fragment for literal billing fixture values;
- serialized `data.js` contains the new fields and remains script-safe.

Frontend unit tests will prove:

- local date formatting for explicit locales, leap days, invalid dates, and timezone-safe component construction;
- graph matching rank, stable ties, result limit, and collapsed-ancestor expansion;
- glossary consumer type filtering and deterministic sorting;
- strict live-payload validation accepts the new contract and rejects missing or malformed statement/scenario fragments.

The release gate will run:

```text
npm test -- --run
npm run typecheck
npm run build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Responsive acceptance will inspect representative desktop and mobile widths when a browser surface is available. Automated tests and production builds remain mandatory even if interactive visual inspection is unavailable.

## Release

The workspace version becomes 0.10.0. `Cargo.lock`, CLI version assertions, contract documentation, and the embedded development snapshot version will agree with the workspace manifest.

After all verification passes, implementation is committed, followed by a release-preparation commit if version metadata is not already part of the final implementation commit. An annotated `v0.10.0` tag is created at the verified release commit. The main branch and tag are pushed to `origin`; the existing tag-triggered GitHub workflow verifies tag/version agreement, rebuilds and tests the embedded frontend, builds platform archives, publishes checksums, and creates generated release notes.

The release is complete only when the pushed tag points to the verified 0.10.0 commit and the GitHub release workflow has been checked for a successful publication outcome.
