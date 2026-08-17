//go:generate go run ../tools/gen-bundle

package bundle

import "embed"

// FS contains every agent-facing asset shipped with the Telos binary.
//
//go:embed skills adapters hooks templates instructions
var FS embed.FS
