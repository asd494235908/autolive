//go:build postgres_integration

package store

import (
	"bufio"
	"context"
	"database/sql"
	"encoding/binary"
	"errors"
	"io"
	"net"
	"net/url"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestPostgresNormalizedActivationRejectsConflictingSessionBindingWithoutResidue(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	fixture := seedPostgresActivationFixture(t, database, ctx, now, controlplane.ActivationCodeStatusActive, now.Add(time.Hour), 1)
	conflictingDeviceID := fixture.DeviceIDs[0] + "_existing"
	if _, err := database.ExecContext(ctx, `
		INSERT INTO devices (id, user_id, device_key, device_name, platform, client_version, status)
		VALUES ($1, $2, $3, $4, $5, $6, $7)
	`, conflictingDeviceID, fixture.UserID, conflictingDeviceID+"-key", "existing-device", "integration", "test", controlplane.DeviceStatusActive); err != nil {
		t.Fatalf("seed conflicting device: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		UPDATE auth_sessions
		SET device_id = $2, device_bound_at = $3
		WHERE access_token_hash = $1
	`, fixture.AccessTokenHashes[0], conflictingDeviceID, now); err != nil {
		t.Fatalf("seed conflicting session binding: %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM devices WHERE id = $1`, conflictingDeviceID)
	})

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("repository constructor: %v", err)
	}
	idempotencyKey := fixture.IdempotencyPrefix + "binding-conflict"
	auditRequestID := fixture.IdempotencyPrefix + "binding-conflict-audit"
	_, err = repository.ActivateDeviceWithSessionBinding(ctx, DeviceActivationRecord{
		Scope:              "control-plane-state",
		IdempotencyKey:     idempotencyKey,
		Fingerprint:        fixture.IdempotencyPrefix + "binding-conflict-fingerprint",
		AccessTokenHash:    fixture.AccessTokenHashes[0],
		UserID:             fixture.UserID,
		ActivationCodeHash: fixture.CodeHash,
		Device: controlplane.DeviceRegistration{
			DeviceID: fixture.DeviceIDs[0], DeviceName: "conflicting-device",
			Platform: "integration", AppVersion: "test",
		},
		Audit: controlplane.AuditLogInput{
			ActorUserID: fixture.UserID, DeviceID: fixture.DeviceIDs[0], Action: "POST /api/v1/client/activate",
			TargetType: "device", TargetID: fixture.DeviceIDs[0], Outcome: "success", StatusCode: 200, RequestID: auditRequestID,
		},
	})
	if !errors.Is(err, ErrSessionDeviceBindingConflict) {
		t.Fatalf("activation error = %v, want session binding conflict", err)
	}

	assertPostgresActivationAttemptUnchanged(t, database, ctx, fixture, idempotencyKey, auditRequestID, conflictingDeviceID)
}

func TestPostgresNormalizedActivationCancellationWhileWaitingForMutationLockLeavesNoResidue(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	fixture := seedPostgresActivationFixture(t, database, ctx, now, controlplane.ActivationCodeStatusActive, now.Add(time.Hour), 1)

	lockConnection, err := database.Conn(ctx)
	if err != nil {
		t.Fatalf("reserve advisory-lock connection: %v", err)
	}
	t.Cleanup(func() { _ = lockConnection.Close() })
	lockTransaction, err := lockConnection.BeginTx(ctx, nil)
	if err != nil {
		t.Fatalf("begin advisory-lock transaction: %v", err)
	}
	t.Cleanup(func() { _ = lockTransaction.Rollback() })
	if _, err := lockTransaction.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended('autolive.control_plane.normalized', 0))`); err != nil {
		t.Fatalf("hold normalized mutation lock: %v", err)
	}
	var holderPID, lockClassID, lockObjectID int64
	if err := lockTransaction.QueryRowContext(ctx, `
		SELECT pid::bigint, classid::bigint, objid::bigint
		FROM pg_locks
		WHERE pid = pg_backend_pid() AND locktype = 'advisory' AND granted
		LIMIT 1
	`).Scan(&holderPID, &lockClassID, &lockObjectID); err != nil {
		t.Fatalf("read held advisory lock identity: %v", err)
	}

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSourceAndTimeout(database, func() time.Time { return now }, nil, ModelReadSourceNormalized, 5*time.Second)
	if err != nil {
		t.Fatalf("repository constructor: %v", err)
	}
	idempotencyKey := fixture.IdempotencyPrefix + "cancelled"
	auditRequestID := fixture.IdempotencyPrefix + "cancelled-audit"
	attemptCtx, cancelAttempt := context.WithCancel(ctx)
	defer cancelAttempt()
	result := make(chan error, 1)
	go func() {
		_, activationErr := repository.ActivateDeviceWithSessionBinding(attemptCtx, DeviceActivationRecord{
			Scope:              "control-plane-state",
			IdempotencyKey:     idempotencyKey,
			Fingerprint:        fixture.IdempotencyPrefix + "cancelled-fingerprint",
			AccessTokenHash:    fixture.AccessTokenHashes[0],
			UserID:             fixture.UserID,
			ActivationCodeHash: fixture.CodeHash,
			Device: controlplane.DeviceRegistration{
				DeviceID: fixture.DeviceIDs[0], DeviceName: "cancelled-device",
				Platform: "integration", AppVersion: "test",
			},
			Audit: controlplane.AuditLogInput{
				ActorUserID: fixture.UserID, DeviceID: fixture.DeviceIDs[0], Action: "POST /api/v1/client/activate",
				TargetType: "device", TargetID: fixture.DeviceIDs[0], Outcome: "success", StatusCode: 200, RequestID: auditRequestID,
			},
		})
		result <- activationErr
	}()

	waitForPostgresAdvisoryLockWait(t, database, ctx, holderPID, lockClassID, lockObjectID)
	cancelAttempt()
	waitCtx, waitCancel := context.WithTimeout(ctx, 2*time.Second)
	defer waitCancel()
	select {
	case activationErr := <-result:
		if !errors.Is(activationErr, context.Canceled) {
			t.Fatalf("cancelled activation error = %v, want context.Canceled", activationErr)
		}
	case <-waitCtx.Done():
		t.Fatalf("cancelled activation did not return: %v", waitCtx.Err())
	}
	if err := lockTransaction.Rollback(); err != nil && !errors.Is(err, sql.ErrTxDone) {
		t.Fatalf("release normalized mutation lock: %v", err)
	}

	assertPostgresActivationAttemptUnchanged(t, database, ctx, fixture, idempotencyKey, auditRequestID, "")
}

func TestPostgresNormalizedActivationConnectionTerminationWhileWaitingLeavesNoResidue(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	fixture := seedPostgresActivationFixture(t, database, ctx, now, controlplane.ActivationCodeStatusActive, now.Add(time.Hour), 1)

	lockConnection, err := database.Conn(ctx)
	if err != nil {
		t.Fatalf("reserve advisory-lock connection: %v", err)
	}
	t.Cleanup(func() { _ = lockConnection.Close() })
	lockTransaction, err := lockConnection.BeginTx(ctx, nil)
	if err != nil {
		t.Fatalf("begin advisory-lock transaction: %v", err)
	}
	t.Cleanup(func() { _ = lockTransaction.Rollback() })
	if _, err := lockTransaction.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended('autolive.control_plane.normalized', 0))`); err != nil {
		t.Fatalf("hold normalized mutation lock: %v", err)
	}
	var holderPID, lockClassID, lockObjectID int64
	if err := lockTransaction.QueryRowContext(ctx, `
		SELECT pid::bigint, classid::bigint, objid::bigint
		FROM pg_locks
		WHERE pid = pg_backend_pid() AND locktype = 'advisory' AND granted
		LIMIT 1
	`).Scan(&holderPID, &lockClassID, &lockObjectID); err != nil {
		t.Fatalf("read held advisory lock identity: %v", err)
	}

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSourceAndTimeout(database, func() time.Time { return now }, nil, ModelReadSourceNormalized, 5*time.Second)
	if err != nil {
		t.Fatalf("repository constructor: %v", err)
	}
	idempotencyKey := fixture.IdempotencyPrefix + "terminated"
	auditRequestID := fixture.IdempotencyPrefix + "terminated-audit"
	attemptCtx, cancelAttempt := context.WithCancel(ctx)
	defer cancelAttempt()
	result := make(chan error, 1)
	go func() {
		_, activationErr := repository.ActivateDeviceWithSessionBinding(attemptCtx, DeviceActivationRecord{
			Scope:              "control-plane-state",
			IdempotencyKey:     idempotencyKey,
			Fingerprint:        fixture.IdempotencyPrefix + "terminated-fingerprint",
			AccessTokenHash:    fixture.AccessTokenHashes[0],
			UserID:             fixture.UserID,
			ActivationCodeHash: fixture.CodeHash,
			Device: controlplane.DeviceRegistration{
				DeviceID: fixture.DeviceIDs[0], DeviceName: "terminated-device",
				Platform: "integration", AppVersion: "test",
			},
			Audit: controlplane.AuditLogInput{
				ActorUserID: fixture.UserID, DeviceID: fixture.DeviceIDs[0], Action: "POST /api/v1/client/activate",
				TargetType: "device", TargetID: fixture.DeviceIDs[0], Outcome: "success", StatusCode: 200, RequestID: auditRequestID,
			},
		})
		result <- activationErr
	}()

	waiterPID := waitForPostgresAdvisoryLockWaitPID(t, database, ctx, holderPID, lockClassID, lockObjectID)
	terminatorCtx, cancelTerminator := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancelTerminator()
	terminatorConnection, err := database.Conn(terminatorCtx)
	if err != nil {
		t.Fatalf("reserve termination connection: %v", err)
	}
	defer terminatorConnection.Close()
	var terminated bool
	if err := terminatorConnection.QueryRowContext(terminatorCtx, `SELECT pg_terminate_backend($1)`, waiterPID).Scan(&terminated); err != nil {
		t.Fatalf("terminate waiting backend pid %d: %v", waiterPID, err)
	}
	if !terminated {
		t.Fatalf("pg_terminate_backend(%d) returned false", waiterPID)
	}

	waitCtx, waitCancel := context.WithTimeout(ctx, 2*time.Second)
	defer waitCancel()
	select {
	case activationErr := <-result:
		if activationErr == nil {
			t.Fatal("activation after backend termination error = nil, want connection failure")
		}
		if errors.Is(activationErr, ErrCommitOutcomeUnknown) {
			t.Fatalf("pre-commit backend termination classified as unknown commit outcome: %v", activationErr)
		}
	case <-waitCtx.Done():
		t.Fatalf("activation did not return after backend termination: %v", waitCtx.Err())
	}
	if err := lockTransaction.Rollback(); err != nil && !errors.Is(err, sql.ErrTxDone) {
		t.Fatalf("release normalized mutation lock: %v", err)
	}

	assertPostgresActivationAttemptUnchanged(t, database, ctx, fixture, idempotencyKey, auditRequestID, "")
}

func TestPostgresNormalizedActivationCommitResponseLossPreservesDurableState(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	now := time.Now().UTC().Truncate(time.Microsecond)
	fixture := seedPostgresActivationFixture(t, database, ctx, now, controlplane.ActivationCodeStatusActive, now.Add(time.Hour), 1)

	databaseURL := os.Getenv("TEST_POSTGRES_URL")
	proxy, err := newPostgresCommitDropProxy(t, databaseURL)
	if err != nil {
		t.Fatalf("create postgres commit-drop proxy: %v", err)
	}
	defer proxy.Close()
	proxiedURL, err := proxy.databaseURL(databaseURL)
	if err != nil {
		t.Fatalf("rewrite proxied postgres URL: %v", err)
	}
	proxiedDatabase, err := sql.Open("postgres", proxiedURL)
	if err != nil {
		t.Fatalf("open proxied postgres database: %v", err)
	}
	defer proxiedDatabase.Close()
	proxiedDatabase.SetMaxOpenConns(2)
	proxiedDatabase.SetMaxIdleConns(1)
	pingCtx, pingCancel := context.WithTimeout(ctx, 2*time.Second)
	if err := proxiedDatabase.PingContext(pingCtx); err != nil {
		pingCancel()
		t.Fatalf("ping proxied postgres database: %v", err)
	}
	pingCancel()

	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSourceAndTimeout(proxiedDatabase, func() time.Time { return now }, nil, ModelReadSourceNormalized, 5*time.Second)
	if err != nil {
		t.Fatalf("repository constructor: %v", err)
	}
	idempotencyKey := fixture.IdempotencyPrefix + "commit-response-loss"
	auditRequestID := fixture.IdempotencyPrefix + "commit-response-loss-audit"
	proxy.ArmCommitDrop()
	_, err = repository.ActivateDeviceWithSessionBinding(ctx, DeviceActivationRecord{
		Scope:              "control-plane-state",
		IdempotencyKey:     idempotencyKey,
		Fingerprint:        fixture.IdempotencyPrefix + "commit-response-loss-fingerprint",
		AccessTokenHash:    fixture.AccessTokenHashes[0],
		UserID:             fixture.UserID,
		ActivationCodeHash: fixture.CodeHash,
		Device: controlplane.DeviceRegistration{
			DeviceID: fixture.DeviceIDs[0], DeviceName: "commit-response-loss-device",
			Platform: "integration", AppVersion: "test",
		},
		Audit: controlplane.AuditLogInput{
			ActorUserID: fixture.UserID, DeviceID: fixture.DeviceIDs[0], Action: "POST /api/v1/client/activate",
			TargetType: "device", TargetID: fixture.DeviceIDs[0], Outcome: "success", StatusCode: 200, RequestID: auditRequestID,
		},
	})
	if !errors.Is(err, ErrCommitOutcomeUnknown) {
		t.Fatalf("activation error = %v, want ErrCommitOutcomeUnknown after committed response loss", err)
	}
	select {
	case <-proxy.Dropped():
	case <-time.After(2 * time.Second):
		t.Fatal("commit-drop proxy did not drop the COMMIT response")
	}

	assertPostgresActivationAttemptCommitted(t, database, ctx, fixture, idempotencyKey, auditRequestID)
}

func waitForPostgresAdvisoryLockWait(t *testing.T, database *sql.DB, ctx context.Context, holderPID, lockClassID, lockObjectID int64) {
	t.Helper()
	waitCtx, waitCancel := context.WithTimeout(ctx, 2*time.Second)
	defer waitCancel()
	ticker := time.NewTicker(10 * time.Millisecond)
	defer ticker.Stop()
	for {
		var waiting bool
		if err := database.QueryRowContext(waitCtx, `
			SELECT EXISTS (
				SELECT 1
				FROM pg_locks
				WHERE locktype = 'advisory'
				  AND NOT granted
				  AND pid <> $1
				  AND classid::bigint = $2
				  AND objid::bigint = $3
			)
		`, holderPID, lockClassID, lockObjectID).Scan(&waiting); err != nil {
			t.Fatalf("observe waiting advisory lock: %v", err)
		}
		if waiting {
			return
		}
		select {
		case <-ticker.C:
		case <-waitCtx.Done():
			t.Fatalf("repository did not wait for normalized mutation lock: %v", waitCtx.Err())
		}
	}
}

func waitForPostgresAdvisoryLockWaitPID(t *testing.T, database *sql.DB, ctx context.Context, holderPID, lockClassID, lockObjectID int64) int64 {
	t.Helper()
	waitCtx, waitCancel := context.WithTimeout(ctx, 2*time.Second)
	defer waitCancel()
	ticker := time.NewTicker(10 * time.Millisecond)
	defer ticker.Stop()
	for {
		var waiterPID sql.NullInt64
		err := database.QueryRowContext(waitCtx, `
			SELECT pid::bigint
			FROM pg_locks
			WHERE locktype = 'advisory'
			  AND NOT granted
			  AND pid <> $1
			  AND classid::bigint = $2
			  AND objid::bigint = $3
			LIMIT 1
		`, holderPID, lockClassID, lockObjectID).Scan(&waiterPID)
		if err == nil && waiterPID.Valid {
			return waiterPID.Int64
		}
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			t.Fatalf("observe waiting advisory lock pid: %v", err)
		}
		select {
		case <-ticker.C:
		case <-waitCtx.Done():
			t.Fatalf("repository did not expose a waiting backend pid: %v", waitCtx.Err())
		}
	}
}

// postgresCommitDropProxy is a test-only PostgreSQL wire fault injector. It
// forwards startup/query frames unchanged and drops the connection after the
// server has acknowledged COMMIT with CommandComplete but before ReadyForQuery.
type postgresCommitDropProxy struct {
	listener      net.Listener
	target        string
	armed         atomic.Bool
	dropped       chan struct{}
	dropOnce      sync.Once
	connectionsMu sync.Mutex
	connections   map[net.Conn]struct{}
	closeOnce     sync.Once
	waitGroup     sync.WaitGroup
}

func newPostgresCommitDropProxy(t *testing.T, databaseURL string) (*postgresCommitDropProxy, error) {
	t.Helper()
	parsed, err := url.Parse(databaseURL)
	if err != nil {
		return nil, err
	}
	if parsed.Host == "" {
		return nil, errors.New("postgres integration URL has no host")
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return nil, err
	}
	proxy := &postgresCommitDropProxy{
		listener:    listener,
		target:      parsed.Host,
		dropped:     make(chan struct{}),
		connections: make(map[net.Conn]struct{}),
	}
	proxy.waitGroup.Add(1)
	go proxy.acceptLoop()
	return proxy, nil
}

func (p *postgresCommitDropProxy) databaseURL(databaseURL string) (string, error) {
	parsed, err := url.Parse(databaseURL)
	if err != nil {
		return "", err
	}
	parsed.Host = p.listener.Addr().String()
	query := parsed.Query()
	query.Set("sslmode", "disable")
	parsed.RawQuery = query.Encode()
	return parsed.String(), nil
}

func (p *postgresCommitDropProxy) ArmCommitDrop() { p.armed.Store(true) }

func (p *postgresCommitDropProxy) Dropped() <-chan struct{} { return p.dropped }

func (p *postgresCommitDropProxy) acceptLoop() {
	defer p.waitGroup.Done()
	for {
		client, err := p.listener.Accept()
		if err != nil {
			return
		}
		server, err := net.Dial("tcp", p.target)
		if err != nil {
			_ = client.Close()
			continue
		}
		p.register(client)
		p.register(server)
		p.waitGroup.Add(2)
		go p.forwardClientToServer(client, server)
		go p.forwardServerToClient(client, server)
	}
}

func (p *postgresCommitDropProxy) forwardClientToServer(client, server net.Conn) {
	defer p.waitGroup.Done()
	defer p.closePair(client, server)
	_, _ = io.Copy(server, client)
}

func (p *postgresCommitDropProxy) forwardServerToClient(client, server net.Conn) {
	defer p.waitGroup.Done()
	defer p.closePair(client, server)
	reader := bufio.NewReader(server)
	for {
		messageType, err := reader.ReadByte()
		if err != nil {
			return
		}
		lengthBytes := make([]byte, 4)
		if _, err := io.ReadFull(reader, lengthBytes); err != nil {
			return
		}
		length := binary.BigEndian.Uint32(lengthBytes)
		if length < 4 || length > 16<<20 {
			return
		}
		body := make([]byte, length-4)
		if _, err := io.ReadFull(reader, body); err != nil {
			return
		}
		frame := make([]byte, 1+4+len(body))
		frame[0] = messageType
		copy(frame[1:5], lengthBytes)
		copy(frame[5:], body)
		if p.armed.Load() && messageType == 'C' && strings.HasPrefix(string(body), "COMMIT") {
			_, _ = client.Write(frame)
			p.dropOnce.Do(func() { close(p.dropped) })
			return
		}
		if _, err := client.Write(frame); err != nil {
			return
		}
	}
}

func (p *postgresCommitDropProxy) register(connection net.Conn) {
	p.connectionsMu.Lock()
	defer p.connectionsMu.Unlock()
	p.connections[connection] = struct{}{}
}

func (p *postgresCommitDropProxy) unregister(connection net.Conn) {
	p.connectionsMu.Lock()
	defer p.connectionsMu.Unlock()
	delete(p.connections, connection)
}

func (p *postgresCommitDropProxy) closePair(client, server net.Conn) {
	_ = client.Close()
	_ = server.Close()
	p.unregister(client)
	p.unregister(server)
}

func (p *postgresCommitDropProxy) Close() {
	p.closeOnce.Do(func() {
		_ = p.listener.Close()
		p.connectionsMu.Lock()
		for connection := range p.connections {
			_ = connection.Close()
		}
		p.connectionsMu.Unlock()
		p.waitGroup.Wait()
	})
}

func assertPostgresActivationAttemptUnchanged(t *testing.T, database *sql.DB, ctx context.Context, fixture postgresActivationFixture, idempotencyKey, auditRequestID, expectedSessionDeviceID string) {
	t.Helper()
	var codeStatus string
	var usedAt sql.NullTime
	var usedUserID, usedDeviceID sql.NullString
	if err := database.QueryRowContext(ctx, `
		SELECT status, used_at, used_by_user_id, used_by_device_id
		FROM activation_codes
		WHERE id = $1
	`, fixture.CodeID).Scan(&codeStatus, &usedAt, &usedUserID, &usedDeviceID); err != nil {
		t.Fatalf("read activation code after rejected attempt: %v", err)
	}
	if codeStatus != controlplane.ActivationCodeStatusActive || usedAt.Valid || usedUserID.Valid || usedDeviceID.Valid {
		t.Fatalf("activation code after rejected attempt = status %q used_at %v user %v device %v", codeStatus, usedAt, usedUserID, usedDeviceID)
	}

	var sessionDeviceID sql.NullString
	if err := database.QueryRowContext(ctx, `SELECT device_id FROM auth_sessions WHERE access_token_hash = $1`, fixture.AccessTokenHashes[0]).Scan(&sessionDeviceID); err != nil {
		t.Fatalf("read session after rejected activation: %v", err)
	}
	if expectedSessionDeviceID == "" {
		if sessionDeviceID.Valid {
			t.Fatalf("session device after rejected activation = %q, want NULL", sessionDeviceID.String)
		}
	} else if !sessionDeviceID.Valid || sessionDeviceID.String != expectedSessionDeviceID {
		t.Fatalf("session device after rejected activation = %v, want %q", sessionDeviceID, expectedSessionDeviceID)
	}

	var deviceCount, idempotencyCount, outboxCount, auditCount int
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM devices WHERE id = $1`, fixture.DeviceIDs[0]).Scan(&deviceCount); err != nil {
		t.Fatalf("count target device after rejected activation: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM idempotency_records WHERE scope = $1 AND idempotency_key = $2`, "control-plane-state", idempotencyKey).Scan(&idempotencyCount); err != nil {
		t.Fatalf("count idempotency records after rejected activation: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM audit_outbox WHERE dedupe_key = $1`, "audit-request:"+auditRequestID).Scan(&outboxCount); err != nil {
		t.Fatalf("count audit outbox after rejected activation: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM audit_logs WHERE request_id = $1`, auditRequestID).Scan(&auditCount); err != nil {
		t.Fatalf("count audit logs after rejected activation: %v", err)
	}
	if deviceCount != 0 || idempotencyCount != 0 || outboxCount != 0 || auditCount != 0 {
		t.Fatalf("rejected activation residue = devices %d idempotency %d outbox %d audit %d, want all zero", deviceCount, idempotencyCount, outboxCount, auditCount)
	}
}

func assertPostgresActivationAttemptCommitted(t *testing.T, database *sql.DB, ctx context.Context, fixture postgresActivationFixture, idempotencyKey, auditRequestID string) {
	t.Helper()
	var codeStatus string
	var usedAt sql.NullTime
	var usedUserID, usedDeviceID sql.NullString
	if err := database.QueryRowContext(ctx, `
		SELECT status, used_at, used_by_user_id, used_by_device_id
		FROM activation_codes
		WHERE id = $1
	`, fixture.CodeID).Scan(&codeStatus, &usedAt, &usedUserID, &usedDeviceID); err != nil {
		t.Fatalf("read activation code after committed response loss: %v", err)
	}
	if codeStatus != controlplane.ActivationCodeStatusUsed || !usedAt.Valid || !usedUserID.Valid || usedUserID.String != fixture.UserID || !usedDeviceID.Valid || usedDeviceID.String != fixture.DeviceIDs[0] {
		t.Fatalf("activation code after committed response loss = status %q used_at %v user %v device %v", codeStatus, usedAt, usedUserID, usedDeviceID)
	}

	var sessionDeviceID sql.NullString
	if err := database.QueryRowContext(ctx, `SELECT device_id FROM auth_sessions WHERE access_token_hash = $1`, fixture.AccessTokenHashes[0]).Scan(&sessionDeviceID); err != nil {
		t.Fatalf("read session after committed response loss: %v", err)
	}
	if !sessionDeviceID.Valid || sessionDeviceID.String != fixture.DeviceIDs[0] {
		t.Fatalf("session device after committed response loss = %v, want %q", sessionDeviceID, fixture.DeviceIDs[0])
	}

	var deviceCount, idempotencyCount, outboxCount, auditCount int
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM devices WHERE id = $1 AND user_id = $2 AND status = $3`, fixture.DeviceIDs[0], fixture.UserID, controlplane.DeviceStatusActive).Scan(&deviceCount); err != nil {
		t.Fatalf("count committed target device: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM idempotency_records WHERE scope = $1 AND idempotency_key = $2`, "control-plane-state", idempotencyKey).Scan(&idempotencyCount); err != nil {
		t.Fatalf("count committed idempotency record: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM audit_outbox WHERE dedupe_key = $1`, "audit-request:"+auditRequestID).Scan(&outboxCount); err != nil {
		t.Fatalf("count committed audit outbox: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM audit_logs WHERE request_id = $1`, auditRequestID).Scan(&auditCount); err != nil {
		t.Fatalf("count committed audit log: %v", err)
	}
	if deviceCount != 1 || idempotencyCount != 1 || outboxCount != 1 || auditCount != 0 {
		t.Fatalf("committed activation facts = device %d idempotency %d outbox %d audit %d, want device/idempotency/outbox 1 and audit 0 before dispatcher", deviceCount, idempotencyCount, outboxCount, auditCount)
	}
}
