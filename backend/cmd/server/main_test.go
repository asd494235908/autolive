package main

import (
	"context"
	"encoding/base64"
	"log/slog"
	"net"
	"net/http"
	"testing"
	"time"

	"autoLive/backend/internal/config"
	"autoLive/backend/internal/httpapi"
	"autoLive/backend/migrations"
)

func TestOpenStorageUsesMemoryOnlyWhenExplicitlyConfigured(t *testing.T) {
	memory, err := openStorage(config.Config{StorageMode: config.StorageModeMemory})
	if err != nil {
		t.Fatalf("openStorage(memory) error = %v", err)
	}
	if memory.repository == nil || memory.secretStore == nil {
		t.Fatal("memory storage did not create repository and secret store")
	}

	_, err = openStorage(config.Config{StorageMode: config.StorageModePostgres, DatabaseURL: "postgres://invalid"})
	if err == nil {
		t.Fatal("openStorage(postgres) error = nil, want missing key error")
	}
}

func TestParseSecretEncryptionKeyAcceptsHexAndBase64(t *testing.T) {
	hexKey := "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"
	key, err := parseSecretEncryptionKey(hexKey)
	if err != nil || len(key) != 32 {
		t.Fatalf("parse hex key = (%d, %v), want 32 bytes and nil error", len(key), err)
	}
	base64Key := base64.RawStdEncoding.EncodeToString(key)
	decoded, err := parseSecretEncryptionKey(base64Key)
	if err != nil || string(decoded) != string(key) {
		t.Fatalf("parse base64 key = (%d, %v), want original key", len(decoded), err)
	}
	if _, err := parseSecretEncryptionKey("short"); err == nil {
		t.Fatal("parse short key error = nil")
	}
}

func TestValidateAppliedMigrationRejectsDirtyOrMismatchedVersion(t *testing.T) {
	if err := validateAppliedMigration(migrations.LatestVersion, false); err != nil {
		t.Fatalf("validateAppliedMigration(current) error = %v", err)
	}
	if err := validateAppliedMigration(migrations.LatestVersion, true); err == nil {
		t.Fatal("validateAppliedMigration(dirty) error = nil")
	}
	if err := validateAppliedMigration(1, false); err == nil {
		t.Fatal("validateAppliedMigration(old version) error = nil")
	}
}

func TestServerGracefulShutdown(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("net.Listen() error = %v", err)
	}

	server := &http.Server{
		Handler: httpapi.NewRouter("test", slog.Default()),
	}
	serveErr := make(chan error, 1)
	go func() { serveErr <- server.Serve(listener) }()

	client := &http.Client{Timeout: time.Second}
	response, err := client.Get("http://" + listener.Addr().String() + "/api/v1/health")
	if err != nil {
		t.Fatalf("health request error = %v", err)
	}
	response.Body.Close()

	shutdownCtx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	if err := server.Shutdown(shutdownCtx); err != nil {
		t.Fatalf("server.Shutdown() error = %v", err)
	}

	if err := <-serveErr; err != http.ErrServerClosed {
		t.Fatalf("Serve() error = %v, want %v", err, http.ErrServerClosed)
	}
}

func TestServerGracefulShutdownWaitsForInFlightRequest(t *testing.T) {
	requestStarted := make(chan struct{})
	releaseRequest := make(chan struct{})
	handler := http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		close(requestStarted)
		<-releaseRequest
		response.WriteHeader(http.StatusOK)
		_, _ = response.Write([]byte("ok"))
	})
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("net.Listen() error = %v", err)
	}
	server := &http.Server{Handler: handler}
	serveErr := make(chan error, 1)
	go func() { serveErr <- server.Serve(listener) }()

	client := &http.Client{Timeout: 2 * time.Second}
	requestDone := make(chan error, 1)
	go func() {
		response, requestErr := client.Get("http://" + listener.Addr().String() + "/in-flight")
		if response != nil {
			response.Body.Close()
		}
		requestDone <- requestErr
	}()
	select {
	case <-requestStarted:
	case <-time.After(time.Second):
		t.Fatal("in-flight request did not start")
	}

	shutdownCtx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	shutdownDone := make(chan error, 1)
	go func() { shutdownDone <- server.Shutdown(shutdownCtx) }()
	select {
	case err := <-shutdownDone:
		t.Fatalf("server.Shutdown() returned before in-flight request completed: %v", err)
	case <-time.After(50 * time.Millisecond):
	}
	close(releaseRequest)
	if err := <-requestDone; err != nil {
		t.Fatalf("in-flight request error = %v", err)
	}
	if err := <-shutdownDone; err != nil {
		t.Fatalf("server.Shutdown() error = %v", err)
	}
	if err := <-serveErr; err != http.ErrServerClosed {
		t.Fatalf("Serve() error = %v, want %v", err, http.ErrServerClosed)
	}
	if response, err := client.Get("http://" + listener.Addr().String() + "/after-shutdown"); err == nil {
		response.Body.Close()
		t.Fatal("request after graceful shutdown unexpectedly succeeded")
	}
}

func TestDefaultHTTPServerTimeoutsAreConfigured(t *testing.T) {
	server := configureHTTPServer(&http.Server{})

	if server.ReadHeaderTimeout != 5*time.Second {
		t.Fatalf("ReadHeaderTimeout = %s, want 5s", server.ReadHeaderTimeout)
	}
	if server.ReadTimeout != 30*time.Second {
		t.Fatalf("ReadTimeout = %s, want 30s", server.ReadTimeout)
	}
	if server.WriteTimeout != 60*time.Second {
		t.Fatalf("WriteTimeout = %s, want 60s", server.WriteTimeout)
	}
	if server.IdleTimeout != 120*time.Second {
		t.Fatalf("IdleTimeout = %s, want 120s", server.IdleTimeout)
	}
}
