package kernel

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
)

// SealModeSealed is the default zero-setup seal: an HMAC with a secret
// embedded in the binary. It detects out-of-protocol edits and prevents
// trivial manual rewriting of certificates; it is NOT a boundary against an
// adversary holding the same binary. A future ATTESTED mode (signed
// commits/tags, external attestation) can strengthen this without changing
// the certificate shape.
const SealModeSealed = "SEALED"

const sealAlgorithm = "HMAC-SHA256"

// embeddedSecret is deliberately a build-time constant: the security boundary
// is the transition kernel (Seal only accepts a verified transition), not the
// secrecy of this value.
var embeddedSecret = []byte("telos-v2-seal-8c1f4b76e2a94d0f")

// SealInfo is the seal block of a certificate envelope.
type SealInfo struct {
	Mode string `json:"mode"`
	Algo string `json:"algo"`
	MAC  string `json:"mac"`
}

func sealPayload(raw []byte) SealInfo {
	m := hmac.New(sha256.New, embeddedSecret)
	m.Write(raw)
	return SealInfo{Mode: SealModeSealed, Algo: sealAlgorithm, MAC: hex.EncodeToString(m.Sum(nil))}
}

func sealValid(raw []byte, s SealInfo) bool {
	if s.Mode != SealModeSealed || s.Algo != sealAlgorithm {
		return false
	}
	want, err := hex.DecodeString(s.MAC)
	if err != nil {
		return false
	}
	m := hmac.New(sha256.New, embeddedSecret)
	m.Write(raw)
	return hmac.Equal(m.Sum(nil), want)
}
