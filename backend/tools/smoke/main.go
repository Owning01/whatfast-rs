// Command smoke proves that the Go sidecar speaks the framed protobuf
// contract of `proto/bridge.proto`: it starts the real binary with a
// throwaway data directory, writes one Hello envelope on its stdin, reads
// the answer from its stdout, checks the answer against the contract and
// closes the process with a graceful Shutdown.
//
// The requests go out through `internal/bridge.Codec`, the exact codec the
// sidecar expects, and the answers come in through a frame reader that
// mirrors the one of `internal/bridge.Codec`: the codec of the sidecar only
// accepts requests, because it is the codec of the desktop side.
//
// Usage:
//
//	go run ./tools/smoke <path-to-whatfast-backend[.exe]>
//
// The smoke test never touches the network: a fresh profile has no paired
// device, so the sidecar skips the connection attempt entirely.
package main

import (
	"encoding/binary"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/Owning01/whatfast/backend/gen/bridgev1"
	"github.com/Owning01/whatfast/backend/internal/bridge"
	"google.golang.org/protobuf/proto"
)

// protocolVersion is the version of the contract the sidecar speaks.
const protocolVersion uint32 = 21

// readTimeout bounds the wait for the answer of the sidecar.
const readTimeout = 30 * time.Second

func main() {
	if err := run(); err != nil {
		fmt.Fprintf(os.Stderr, "smoke: %v\n", err)
		os.Exit(1)
	}
}

func run() error {
	if len(os.Args) != 2 {
		return fmt.Errorf("usage: smoke <path-to-whatfast-backend>")
	}
	sidecar, err := filepath.Abs(os.Args[1])
	if err != nil {
		return fmt.Errorf("resolve the sidecar path: %w", err)
	}
	if _, err := os.Stat(sidecar); err != nil {
		return fmt.Errorf("the sidecar is not there: %w", err)
	}
	dataDir, err := os.MkdirTemp("", "whatfast-smoke-")
	if err != nil {
		return fmt.Errorf("create the throwaway data directory: %w", err)
	}
	defer os.RemoveAll(dataDir)

	command := exec.Command(sidecar, "--stdio", "--data-dir", dataDir)
	stdin, err := command.StdinPipe()
	if err != nil {
		return fmt.Errorf("open the sidecar stdin: %w", err)
	}
	stdout, err := command.StdoutPipe()
	if err != nil {
		return fmt.Errorf("open the sidecar stdout: %w", err)
	}
	stderr := &syncBuffer{}
	command.Stderr = stderr
	if err := command.Start(); err != nil {
		return fmt.Errorf("start the sidecar: %w", err)
	}
	// The sidecar is always stopped, even when the check fails.
	defer func() {
		_ = stdin.Close()
		if command.Process != nil {
			_ = command.Process.Kill()
		}
		_ = command.Wait()
	}()

	// The codec writes the requests exactly as the sidecar expects them.
	codec := bridge.NewCodec(stdout, stdin)
	if err := codec.Write(helloEnvelope()); err != nil {
		return fmt.Errorf("write the Hello envelope: %w", err)
	}
	answer, err := readWithTimeout(stdout)
	if err != nil {
		return fmt.Errorf("read the answer of the sidecar: %w%s", err, stderr.tail())
	}
	if err := check(answer); err != nil {
		return fmt.Errorf("the sidecar answered wrong: %w%s", err, stderr.tail())
	}
	fmt.Printf("hello ok: protocol_version=%d backend_version=%q\n",
		answer.GetResponse().GetHello().GetProtocolVersion(),
		answer.GetResponse().GetHello().GetBackendVersion())

	if err := codec.Write(shutdownEnvelope()); err != nil {
		return fmt.Errorf("write the Shutdown envelope: %w", err)
	}
	if _, err := readWithTimeout(stdout); err != nil {
		return fmt.Errorf("read the answer of the Shutdown: %w%s", err, stderr.tail())
	}
	if err := command.Wait(); err != nil {
		return fmt.Errorf("the sidecar did not exit cleanly: %w%s", err, stderr.tail())
	}
	fmt.Println("shutdown ok: the sidecar exited with code 0")
	return nil
}

// helloEnvelope is the first request of every session: the client announces
// the desktop version and the range of protocol versions it speaks.
func helloEnvelope() *bridgev1.Envelope {
	return &bridgev1.Envelope{
		ProtocolVersion: protocolVersion,
		RequestId:       1,
		Body: &bridgev1.Envelope_Request{Request: &bridgev1.RpcRequest{
			Request: &bridgev1.RpcRequest_Hello{Hello: &bridgev1.HelloRequest{
				DesktopVersion:         "smoke",
				MinimumProtocolVersion: protocolVersion,
				MaximumProtocolVersion: protocolVersion,
			}},
		}},
	}
}

// shutdownEnvelope asks the sidecar to stop.
func shutdownEnvelope() *bridgev1.Envelope {
	return &bridgev1.Envelope{
		ProtocolVersion: protocolVersion,
		RequestId:       2,
		Body: &bridgev1.Envelope_Request{Request: &bridgev1.RpcRequest{
			Request: &bridgev1.RpcRequest_Shutdown{Shutdown: &bridgev1.ShutdownRequest{}},
		}},
	}
}

// check compares the answer of the sidecar with the contract.
func check(answer *bridgev1.Envelope) error {
	if answer.GetProtocolVersion() != protocolVersion {
		return fmt.Errorf("the envelope carries the protocol version %d, the contract says %d",
			answer.GetProtocolVersion(), protocolVersion)
	}
	if answer.GetRequestId() != 1 {
		return fmt.Errorf("the envelope answers the request %d, the request was the 1", answer.GetRequestId())
	}
	if failure := answer.GetResponse().GetError(); failure != nil {
		return fmt.Errorf("the sidecar refused the handshake: %s (%s)", failure.GetCode(), failure.GetMessage())
	}
	hello := answer.GetResponse().GetHello()
	if hello == nil {
		return fmt.Errorf("the answer carries no Hello response")
	}
	if hello.GetProtocolVersion() != protocolVersion {
		return fmt.Errorf("the Hello announces the protocol version %d, the contract says %d",
			hello.GetProtocolVersion(), protocolVersion)
	}
	if hello.GetBackendVersion() == "" {
		return fmt.Errorf("the Hello announces no backend version")
	}
	return nil
}

// readFrame reads one frame of the contract: a 4-byte big-endian length and
// the protobuf payload. The framing is the one of `internal/bridge.Codec`
// and the one of the Rust client (`whatfast-protocol`, `read_frame`).
func readFrame(reader io.Reader) (*bridgev1.Envelope, error) {
	var header [4]byte
	if _, err := io.ReadFull(reader, header[:]); err != nil {
		return nil, err
	}
	size := binary.BigEndian.Uint32(header[:])
	if size == 0 || size > bridge.MaxFrameSize {
		return nil, fmt.Errorf("invalid frame size %d", size)
	}
	payload := make([]byte, size)
	if _, err := io.ReadFull(reader, payload); err != nil {
		return nil, err
	}
	var envelope bridgev1.Envelope
	if err := proto.Unmarshal(payload, &envelope); err != nil {
		return nil, fmt.Errorf("decode the envelope: %w", err)
	}
	return &envelope, nil
}

// readWithTimeout reads one frame, or gives up when the sidecar stays quiet.
func readWithTimeout(reader io.Reader) (*bridgev1.Envelope, error) {
	type result struct {
		envelope *bridgev1.Envelope
		err      error
	}
	answers := make(chan result, 1)
	go func() {
		envelope, err := readFrame(reader)
		answers <- result{envelope: envelope, err: err}
	}()
	select {
	case answer := <-answers:
		return answer.envelope, answer.err
	case <-time.After(readTimeout):
		return nil, fmt.Errorf("no answer in %s", readTimeout)
	}
}

// syncBuffer collects the diagnostics of the sidecar without racing on it.
type syncBuffer struct {
	mu    sync.Mutex
	lines []string
}

func (b *syncBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	for _, line := range strings.Split(strings.TrimRight(string(p), "\r\n"), "\n") {
		if line != "" {
			b.lines = append(b.lines, line)
		}
	}
	return len(p), nil
}

// tail returns the diagnostics of the sidecar, ready to be printed.
func (b *syncBuffer) tail() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	if len(b.lines) == 0 {
		return ""
	}
	return "\nsidecar diagnostics:\n  " + strings.Join(b.lines, "\n  ")
}

// The compile-time check that the buffer is usable as an io.Writer.
var _ io.Writer = (*syncBuffer)(nil)
