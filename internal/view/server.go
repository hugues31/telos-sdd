package view

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/hugues31/telos-sdd/internal/coded"
	"github.com/hugues31/telos-sdd/internal/graph"
	"github.com/hugues31/telos-sdd/internal/kernel"
)

// Options configures the view server. There is deliberately no bind option:
// the server is loopback-only by design.
type Options struct {
	Port    int // 0 = ephemeral
	Querier graph.Querier
	Status  func() (kernel.ProjectStatus, error)
	Version string
}

// Handler builds the read-only HTTP handler: GET-only, Host-checked against
// DNS rebinding, self-contained CSP, no mutating endpoint of any kind.
func Handler(opts Options) http.Handler {
	mux := http.NewServeMux()
	s := &site{q: opts.Querier, status: opts.Status, version: opts.Version}
	mux.HandleFunc("/", s.overview)
	mux.HandleFunc("/contract", s.contract)
	mux.HandleFunc("/node/", s.node)
	mux.HandleFunc("/changes", s.changes)
	mux.HandleFunc("/evidence", s.evidence)
	mux.HandleFunc("/findings", s.findings)
	mux.HandleFunc("/graph", s.graphPage)
	mux.HandleFunc("/health", s.health)

	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet && r.Method != http.MethodHead {
			http.Error(w, "the Telos view is read-only", http.StatusMethodNotAllowed)
			return
		}
		host := r.Host
		if h, _, err := net.SplitHostPort(r.Host); err == nil {
			host = h
		}
		if host != "127.0.0.1" && host != "localhost" && host != "::1" {
			http.Error(w, "forbidden host", http.StatusForbidden)
			return
		}
		w.Header().Set("Content-Security-Policy", "default-src 'self'; style-src 'unsafe-inline'; img-src data:")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		mux.ServeHTTP(w, r)
	})
}

// Serve runs the loopback server until interrupted.
func Serve(opts Options, stdout interface{ Write([]byte) (int, error) }) (string, error) {
	listener, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", opts.Port))
	if err != nil {
		return "", coded.New("TELOS_PORT_BUSY", "the view port is taken; pass --port (0 for an ephemeral port): "+err.Error())
	}
	url := "http://" + listener.Addr().String()
	server := &http.Server{Handler: Handler(opts), ReadHeaderTimeout: 5 * time.Second}
	fmt.Fprintf(stdout, "Telos view: %s (Ctrl-C to stop)\n", url)

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	select {
	case <-ctx.Done():
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
		defer cancel()
		_ = server.Shutdown(shutdownCtx)
		return url, nil
	case err := <-done:
		if errors.Is(err, http.ErrServerClosed) {
			return url, nil
		}
		return url, err
	}
}

// StaticExport renders the main pages and every artifact node page into dir.
func StaticExport(opts Options, dir string) ([]string, error) {
	handler := Handler(opts)
	var written []string
	render := func(path, file string) error {
		req, _ := http.NewRequest(http.MethodGet, "http://127.0.0.1"+path, nil)
		rec := &memoryResponse{header: http.Header{}}
		handler.ServeHTTP(rec, req)
		if rec.status != 0 && rec.status != http.StatusOK {
			return fmt.Errorf("%s: HTTP %d", path, rec.status)
		}
		target := dir + "/" + file
		if err := os.MkdirAll(dirOf(target), 0o755); err != nil {
			return err
		}
		if err := os.WriteFile(target, rec.body, 0o644); err != nil {
			return err
		}
		written = append(written, file)
		return nil
	}
	pages := map[string]string{
		"/": "index.html", "/contract": "contract.html", "/changes": "changes.html",
		"/evidence": "evidence.html", "/findings": "findings.html", "/graph": "graph.html",
		"/health": "health.html",
	}
	for path, file := range pages {
		if err := render(path, file); err != nil {
			return written, err
		}
	}
	if opts.Querier != nil {
		nodes, err := opts.Querier.Nodes(graph.NodeFilter{Kinds: []graph.NodeKind{
			graph.KindIntent, graph.KindRequirement, graph.KindDecision, graph.KindChange}})
		if err != nil {
			return written, err
		}
		for _, n := range nodes {
			if err := render("/node/"+string(n.ID), "node/"+string(n.ID)+".html"); err != nil {
				return written, err
			}
		}
	}
	return written, nil
}

func dirOf(path string) string {
	if i := strings.LastIndexByte(path, '/'); i > 0 {
		return path[:i]
	}
	return "."
}

type memoryResponse struct {
	header http.Header
	body   []byte
	status int
}

func (m *memoryResponse) Header() http.Header { return m.header }
func (m *memoryResponse) Write(p []byte) (int, error) {
	m.body = append(m.body, p...)
	return len(p), nil
}
func (m *memoryResponse) WriteHeader(status int) { m.status = status }
