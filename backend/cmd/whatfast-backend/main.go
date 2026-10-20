package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"

	"github.com/Owning01/whatfast/backend/internal/app"
	"github.com/Owning01/whatfast/backend/internal/bridge"
	"github.com/Owning01/whatfast/backend/internal/logging"
	"github.com/Owning01/whatfast/backend/internal/profilelock"
	"github.com/Owning01/whatfast/backend/internal/securefs"
	"github.com/Owning01/whatfast/backend/internal/store"
	"github.com/Owning01/whatfast/backend/internal/wa"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
func run() error {
	var dataDir, logFile string
	var stdio bool
	flag.StringVar(&dataDir, "data-dir", "", "directory for session and client databases")
	flag.StringVar(&logFile, "log-file", "", "optional log path")
	flag.BoolVar(&stdio, "stdio", false, "serve framed protobuf over stdin/stdout")
	flag.Parse()
	if !stdio {
		return fmt.Errorf("--stdio is required")
	}
	if dataDir == "" {
		return fmt.Errorf("--data-dir is required")
	}
	abs, err := filepath.Abs(dataDir)
	if err != nil {
		return err
	}
	if err = securefs.EnsurePrivateDirectory(abs); err != nil {
		return err
	}
	profile, err := profilelock.Acquire(abs)
	if err != nil {
		return err
	}
	defer profile.Close()
	log, closer, err := logging.New(logFile)
	if err != nil {
		return err
	}
	if closer != nil {
		defer closer.Close()
	}
	base, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	ctx, cancel := context.WithCancel(base)
	defer cancel()
	productStore, err := store.Open(ctx, filepath.Join(abs, "client.db"))
	if err != nil {
		return err
	}
	defer productStore.Close()
	codec := bridge.NewCodec(os.Stdin, os.Stdout)
	server := app.New(ctx, cancel, codec, productStore)
	client, err := wa.New(ctx, abs, productStore, server.Emit, log)
	if err != nil {
		return err
	}
	defer client.Close()
	server.SetWhatsApp(client)
	if connect := client.PrepareConnect(); connect != nil {
		go func() {
			if err := connect(); err != nil {
				log.Error("connect failed", "error", err)
			}
		}()
	}
	return server.Run()
}
