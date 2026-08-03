package main

import (
	"fmt"
	"os"

	"github.com/hugues31/telos-sdd/internal/telos"
)

var version = "dev"

func main() {
	if err := telos.Run(os.Args[1:], version, os.Stdin, os.Stdout, os.Stderr); err != nil {
		if !telos.IsReported(err) {
			fmt.Fprintln(os.Stderr, "telos:", err)
		}
		os.Exit(1)
	}
}
