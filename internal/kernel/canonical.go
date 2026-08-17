package kernel

import (
	"bytes"
	"encoding/json"
)

// marshalCanonical produces the canonical byte representation of a value:
// compact JSON with HTML escaping disabled and no trailing newline. Payloads
// are structs and slices only (never maps), so byte determinism follows from
// struct field order. Any schema change bumps the payload version.
func marshalCanonical(v any) ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(v); err != nil {
		return nil, err
	}
	return bytes.TrimRight(buf.Bytes(), "\n"), nil
}
