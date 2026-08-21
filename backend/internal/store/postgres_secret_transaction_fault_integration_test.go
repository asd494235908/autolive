//go:build postgres_integration

package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"
	"strings"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
)

func TestPostgresNormalizedSecretRotationCommitResponseLossIsRecoverable(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	fixture := seedPostgresSecretRotationFixture(t, database, ctx, "commit-loss")
	now := fixture.now
	accountID := fixture.accountID
	oldSecretRef := fixture.oldSecretRef
	idempotencyKey := fixture.idempotencyKey
	auditRequestID := fixture.auditRequestID
	const encryptionKey = "01234567890123456789012345678901"

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

	proxiedSecretStore, err := NewEncryptedSQLSecretStore(proxiedDatabase, []byte(encryptionKey))
	if err != nil {
		t.Fatalf("proxied secret store constructor: %v", err)
	}
	proxiedRepository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSourceAndTimeout(
		proxiedDatabase, func() time.Time { return now }, proxiedSecretStore, ModelReadSourceNormalized, 5*time.Second,
	)
	if err != nil {
		t.Fatalf("proxied repository constructor: %v", err)
	}
	proxy.ArmCommitDrop()
	_, err = proxiedRepository.RotateModelPoolAccountSecret(ctx, ModelPoolSecretRotationRecord{
		Scope:             "model-account-secret-rotation",
		IdempotencyKey:    idempotencyKey,
		Fingerprint:       "rotation-commit-loss-fingerprint",
		AccountID:         accountID,
		ExpectedSecretRef: oldSecretRef,
		APIKey:            "integration-new-secret",
		Probe: controlplane.ModelPoolConnectivityTestResult{
			AccountID: accountID, Provider: "integration", Model: "test-model", Status: "succeeded",
			TestedAt: now.Format(time.RFC3339), LatencyMS: 1, HTTPStatus: 200,
		},
		Audit: controlplane.AuditLogInput{
			Action: "model_account.rotate_secret", TargetType: "model_account", TargetID: accountID,
			Outcome: "success", StatusCode: 200, RequestID: auditRequestID,
		},
	})
	if !errors.Is(err, ErrCommitOutcomeUnknown) {
		t.Fatalf("rotation error = %v, want ErrCommitOutcomeUnknown after committed response loss", err)
	}
	select {
	case <-proxy.Dropped():
	case <-time.After(2 * time.Second):
		t.Fatal("commit-drop proxy did not drop the COMMIT response")
	}

	var activeSecretRef string
	if err := database.QueryRowContext(ctx, `SELECT secret_ref FROM model_accounts WHERE id = $1`, accountID).Scan(&activeSecretRef); err != nil {
		t.Fatalf("read active secret reference after unknown commit: %v", err)
	}
	if !strings.HasPrefix(activeSecretRef, oldSecretRef+"/rotation_") {
		t.Fatalf("active secret reference after unknown commit = %q, want staged rotation reference", activeSecretRef)
	}
	recoveredSecretStore, err := NewEncryptedSQLSecretStore(database, []byte(encryptionKey))
	if err != nil {
		t.Fatalf("recovered secret store constructor: %v", err)
	}
	if stored, err := recoveredSecretStore.Get(ctx, activeSecretRef); err != nil || stored != "integration-new-secret" {
		t.Fatalf("new secret after unknown commit = (%q, %v)", stored, err)
	}
	if _, err := recoveredSecretStore.Get(ctx, oldSecretRef); !errors.Is(err, ErrSecretNotFound) {
		t.Fatalf("old secret after unknown commit lookup error = %v, want ErrSecretNotFound", err)
	}

	recoveredRepository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, recoveredSecretStore, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("recovered repository constructor: %v", err)
	}
	replayed, err := recoveredRepository.RotateModelPoolAccountSecret(ctx, ModelPoolSecretRotationRecord{
		Scope: "model-account-secret-rotation", IdempotencyKey: idempotencyKey, Fingerprint: "rotation-commit-loss-fingerprint",
		AccountID: accountID, ExpectedSecretRef: oldSecretRef, APIKey: "ignored-replay-secret",
		Probe: controlplane.ModelPoolConnectivityTestResult{AccountID: accountID, Status: "succeeded"},
	})
	if err != nil || replayed.ID != accountID {
		t.Fatalf("replayed rotation after recovery = (%+v, %v), want persisted result", replayed, err)
	}
	var idempotencyCount, testCount, outboxCount int
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM idempotency_records WHERE scope = $1 AND idempotency_key = $2`, "model-account-secret-rotation", idempotencyKey).Scan(&idempotencyCount); err != nil {
		t.Fatalf("count rotation idempotency record: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM model_pool_test_results WHERE account_id = $1`, accountID).Scan(&testCount); err != nil {
		t.Fatalf("count rotation test result: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM audit_outbox WHERE dedupe_key = $1`, "audit-request:"+auditRequestID).Scan(&outboxCount); err != nil {
		t.Fatalf("count rotation audit outbox: %v", err)
	}
	if idempotencyCount != 1 || testCount != 1 || outboxCount != 1 {
		t.Fatalf("recovered rotation facts = idempotency %d tests %d outbox %d, want 1/1/1", idempotencyCount, testCount, outboxCount)
	}
}

func TestPostgresNormalizedSecretRotationConnectionTerminationBeforeCommitLeavesNoResidue(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	fixture := seedPostgresSecretRotationFixture(t, database, ctx, "precommit-terminate")

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

	secretStore, err := NewEncryptedSQLSecretStore(database, []byte("01234567890123456789012345678901"))
	if err != nil {
		t.Fatalf("secret store constructor: %v", err)
	}
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSourceAndTimeout(database, func() time.Time { return fixture.now }, secretStore, ModelReadSourceNormalized, 5*time.Second)
	if err != nil {
		t.Fatalf("repository constructor: %v", err)
	}
	result := make(chan error, 1)
	go func() {
		_, rotationErr := repository.RotateModelPoolAccountSecret(ctx, ModelPoolSecretRotationRecord{
			Scope: "model-account-secret-rotation", IdempotencyKey: fixture.idempotencyKey, Fingerprint: "precommit-terminate-fingerprint",
			AccountID: fixture.accountID, ExpectedSecretRef: fixture.oldSecretRef, APIKey: "integration-new-secret",
			Probe: controlplane.ModelPoolConnectivityTestResult{AccountID: fixture.accountID, Status: "succeeded"},
		})
		result <- rotationErr
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
	case rotationErr := <-result:
		if rotationErr == nil {
			t.Fatal("rotation after backend termination error = nil, want connection failure")
		}
		if errors.Is(rotationErr, ErrCommitOutcomeUnknown) {
			t.Fatalf("pre-commit backend termination classified as unknown commit outcome: %v", rotationErr)
		}
	case <-waitCtx.Done():
		t.Fatalf("rotation did not return after backend termination: %v", waitCtx.Err())
	}
	if err := lockTransaction.Rollback(); err != nil && !errors.Is(err, sql.ErrTxDone) {
		t.Fatalf("release normalized mutation lock: %v", err)
	}

	var activeSecretRef string
	if err := database.QueryRowContext(ctx, `SELECT secret_ref FROM model_accounts WHERE id = $1`, fixture.accountID).Scan(&activeSecretRef); err != nil {
		t.Fatalf("read active secret reference after pre-commit termination: %v", err)
	}
	if activeSecretRef != fixture.oldSecretRef {
		t.Fatalf("active secret reference after pre-commit termination = %q, want %q", activeSecretRef, fixture.oldSecretRef)
	}
	if stored, err := secretStore.Get(ctx, fixture.oldSecretRef); err != nil || stored != "integration-old-secret" {
		t.Fatalf("old secret after pre-commit termination = (%q, %v)", stored, err)
	}
	var stagedCount, testCount, idempotencyCount, outboxCount int
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM model_account_secrets WHERE secret_ref LIKE $1`, fixture.oldSecretRef+"/%").Scan(&stagedCount); err != nil {
		t.Fatalf("count staged secrets after pre-commit termination: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM model_pool_test_results WHERE account_id = $1`, fixture.accountID).Scan(&testCount); err != nil {
		t.Fatalf("count test results after pre-commit termination: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM idempotency_records WHERE scope = $1 AND idempotency_key = $2`, "model-account-secret-rotation", fixture.idempotencyKey).Scan(&idempotencyCount); err != nil {
		t.Fatalf("count idempotency records after pre-commit termination: %v", err)
	}
	if err := database.QueryRowContext(ctx, `SELECT COUNT(*) FROM audit_outbox WHERE dedupe_key = $1`, "audit-request:"+fixture.auditRequestID).Scan(&outboxCount); err != nil {
		t.Fatalf("count audit outbox after pre-commit termination: %v", err)
	}
	if stagedCount != 0 || testCount != 0 || idempotencyCount != 0 || outboxCount != 0 {
		t.Fatalf("pre-commit termination residue = staged %d tests %d idempotency %d outbox %d, want all zero", stagedCount, testCount, idempotencyCount, outboxCount)
	}
}

type postgresSecretRotationFixture struct {
	now            time.Time
	accountID      string
	oldSecretRef   string
	idempotencyKey string
	auditRequestID string
}

func seedPostgresSecretRotationFixture(t *testing.T, database *sql.DB, ctx context.Context, label string) postgresSecretRotationFixture {
	t.Helper()
	now := time.Now().UTC().Truncate(time.Microsecond)
	suffix := now.UnixNano()
	accountID := fmt.Sprintf("rotation_%s_account_%d", label, suffix)
	oldSecretRef := "model-account/" + accountID
	idempotencyKey := fmt.Sprintf("rotation-%s:%d", label, suffix)
	auditRequestID := fmt.Sprintf("rotation-%s-audit:%d", label, suffix)
	secretStore, err := NewEncryptedSQLSecretStore(database, []byte("01234567890123456789012345678901"))
	if err != nil {
		t.Fatalf("secret store constructor: %v", err)
	}
	if _, err := database.ExecContext(ctx, `
		INSERT INTO model_accounts (
			id, provider, model, base_url, secret_ref, status, priority,
			concurrency_limit, daily_token_limit, active_requests,
			daily_reserved_tokens, created_at, updated_at
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, 0, $10, $10)
	`, accountID, "integration", "test-model", "https://provider.example.test", oldSecretRef,
		controlplane.ModelAccountStatusActive, 1, 2, 100, now); err != nil {
		t.Fatalf("seed model account: %v", err)
	}
	if err := secretStore.Put(ctx, oldSecretRef, "integration-old-secret"); err != nil {
		t.Fatalf("seed model account secret: %v", err)
	}
	t.Cleanup(func() {
		cleanupCtx, cleanupCancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cleanupCancel()
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM audit_outbox WHERE dedupe_key = $1`, "audit-request:"+auditRequestID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_pool_test_results WHERE account_id = $1`, accountID)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_account_secrets WHERE secret_ref LIKE $1`, oldSecretRef+"/%")
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_account_secrets WHERE secret_ref = $1`, oldSecretRef)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM idempotency_records WHERE scope = $1 AND idempotency_key = $2`, "model-account-secret-rotation", idempotencyKey)
		_, _ = database.ExecContext(cleanupCtx, `DELETE FROM model_accounts WHERE id = $1`, accountID)
	})
	return postgresSecretRotationFixture{now: now, accountID: accountID, oldSecretRef: oldSecretRef, idempotencyKey: idempotencyKey, auditRequestID: auditRequestID}
}
