package telos

// ConsumerPin is the version consumer repositories are pinned to by
// `telos init --ci github` and the published verify action. It is the ONE
// bump site of the release checklist — consumers never track @latest, so a
// new major behavior can never silently reach their CI.
const ConsumerPin = "v0.6.1"
