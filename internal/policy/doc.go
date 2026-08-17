// Package policy loads and evaluates certification policy: an embedded,
// closed kernel schema and floor (non-weakenable by construction — KERNEL-008)
// unified with the project's policies/*.cue. The unified value's canonical
// export is hashed into the certificate. The implementation arrives at M6;
// this package currently pins the CUE dependency, whose unification semantics
// the smoke test verifies.
package policy
