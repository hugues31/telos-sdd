# Contributing

Telos changes should follow Telos itself whenever practical: state the intent, define observable rules, design adversarial tests, and keep implementation within that scope.

Before opening a pull request, run:

```bash
go test ./...
go vet ./...
python3 scripts/validate-skills.py
git diff --check
```

Keep the CLI dependency-free unless a dependency provides clear, durable value that cannot be implemented safely with the Go standard library. Preserve compatibility across Linux, macOS, and Windows. Changes to generated provider files must originate under `bundle/` and include installation tests.

Security issues should follow [SECURITY.md](SECURITY.md), not a public issue.

