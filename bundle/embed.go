package bundle

import "embed"

// FS contains every agent-facing asset shipped with the Telos binary.
//
//go:embed skills adapters hooks templates
var FS embed.FS
