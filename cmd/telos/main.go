package main

import (
	"fmt"
	"os"
	"runtime/debug"

	"github.com/hugues31/telos-sdd/internal/telos"
)

var version = "dev"

func main() {
	// Release binaries get the version via -ldflags; `go install` binaries
	// carry it in their build info instead.
	if version == "dev" {
		if info, ok := debug.ReadBuildInfo(); ok && info.Main.Version != "" && info.Main.Version != "(devel)" {
			version = info.Main.Version
		}
	}
	if err := telos.Run(os.Args[1:], version, os.Stdin, os.Stdout, os.Stderr); err != nil {
		if !telos.IsReported(err) {
			fmt.Fprintln(os.Stderr, "telos:", err)
		}
		os.Exit(1)
	}
}
