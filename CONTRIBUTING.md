# Contributing

Telos changes should follow Telos itself whenever practical: state the intent, define observable rules, design adversarial tests, and keep implementation within that scope.

Before opening a pull request, run:

```bash
go test ./...
go vet ./...
gofmt -l .
python3 scripts/validate-skills.py
git diff --check
```

## Dependency policy

The CLI ships with a deliberately small, explicitly allowed dependency set:

- `modernc.org/sqlite` — derived semantic-graph index (pure Go, FTS5).
- `cuelang.org/go` — certification policies and structured constraints.

Everything else comes from the Go standard library. Adding a dependency is a maintainer decision recorded as a `DEC-*` entry, and any candidate must satisfy all of:

1. clear, durable value that cannot be implemented safely with the standard library;
2. pure Go — the CI cross-compile job must stay green on all six release targets with `CGO_ENABLED=0`;
3. the binary stays within the CI size budget;
4. no surprising transitive licensing.

External binaries may only ever be optional at runtime (`z3` is the model: detected, never required, absence reported explicitly).

Preserve compatibility across Linux, macOS, and Windows. Changes to generated provider files must originate under `bundle/` and include installation tests.

Security issues should follow [SECURITY.md](SECURITY.md), not a public issue.
