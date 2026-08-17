package constraints

import (
	"strings"
	"testing"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/contract"
)

func contractWith(t *testing.T, blocks map[string]string) contract.Contract {
	t.Helper()
	c := contract.Contract{Requirements: map[string]*contract.Requirement{}}
	for id, block := range blocks {
		c.Requirements[id] = &contract.Requirement{ID: id, Constraint: block}
	}
	return c
}

func TestCheckSatisfiable(t *testing.T) {
	c := contractWith(t, map[string]string{
		"REQ-001": "timeout_min: >=5 & <=30",
		"REQ-002": "timeout_min: >=10",
		"REQ-003": "", // unformalized requirements are never an error
	})
	if err := Check(c); err != nil {
		t.Fatal(err)
	}
	if err := Check(contract.Contract{}); err != nil {
		t.Fatal(err)
	}
}

func TestCheckContradiction(t *testing.T) {
	c := contractWith(t, map[string]string{
		"REQ-001": "timeout_min: >=20",
		"REQ-002": "timeout_min: <=10\ntimeout_min: 5",
		"REQ-003": "timeout_min: 25",
	})
	err := Check(c)
	e, ok := coded.As(err)
	if !ok || e.Code != "TELOS_CONSTRAINT_UNSAT" {
		t.Fatalf("err = %v", err)
	}
	if !strings.Contains(strings.Join(e.Paths, ","), "REQ-001") {
		t.Fatalf("paths = %v", e.Paths)
	}
}

func TestCheckInvalidBlock(t *testing.T) {
	c := contractWith(t, map[string]string{"REQ-001": "not valid cue {{{"})
	err := Check(c)
	e, ok := coded.As(err)
	if !ok || e.Code != "TELOS_CONTRACT_INVALID" {
		t.Fatalf("err = %v", err)
	}
}
