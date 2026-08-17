# Release checklist — v0.6.0

One bump site, then verify outward from the kernel:

1. `internal/telos/versionpin.go` — `ConsumerPin` matches the tag about to
   be cut (consumer CI templates and the verify action pin to it).
2. Local gates green:
   `go test ./...` · `go vet ./...` · `gofmt -l .` empty ·
   `go generate ./bundle && git diff --exit-code` ·
   `go run ./tools/gen-bundle -check`.
3. CI green on all three OSes AND the cross-compile job (six targets,
   `CGO_ENABLED=0`, size budget) — this is what proves the pure-Go claim of
   the sqlite/CUE dependencies on release day.
4. Dogfooding gate: `telos init` on a scratch clone of telos-sdd itself,
   run the three loops by hand, `telos verify` green. (Full self-hosting of
   the development repo is the first roadmap item after the tag.)
5. Tag `v0.6.0` and push: `release.yml` builds the six archives with
   checksums and creates the GitHub release.
6. Smoke the installers against the draft release (`install.sh`,
   `install.ps1`, `go install ...@v0.6.0`).
7. Release notes: this is a clean-break rewrite (V1's spec_pending,
   annotations, state.json, and the spec/apply verbs are gone); no migration
   tooling ships because no released version had users.
