# Security policy

Please do not disclose a vulnerability in a public issue. Send a private report through GitHub’s security-advisory interface for this repository and include affected versions, reproduction steps, impact, and any suggested mitigation.

Telos hooks are defense-in-depth guardrails and the view server is loopback-only and read-only. Hash verification detects mutation; it does not sandbox the host, authenticate the human operator, or make an untrusted repository safe to execute.

