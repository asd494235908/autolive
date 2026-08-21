package store

import (
	"bytes"
	"context"
	"database/sql"
	"errors"
	"regexp"
	"testing"
	"time"

	"github.com/DATA-DOG/go-sqlmock"
	"github.com/lib/pq"
)

func TestSecretCiphertextRoundTripDoesNotContainPlaintext(t *testing.T) {
	key := bytes.Repeat([]byte{0x42}, 32)
	plaintext := []byte("sk-sensitive-value")
	ciphertext, err := encryptSecret(key, plaintext)
	if err != nil {
		t.Fatalf("encryptSecret() error = %v", err)
	}
	if bytes.Contains(ciphertext, plaintext) {
		t.Fatal("ciphertext contains plaintext secret")
	}
	decoded, err := decryptSecret(key, ciphertext)
	if err != nil {
		t.Fatalf("decryptSecret() error = %v", err)
	}
	if !bytes.Equal(decoded, plaintext) {
		t.Fatalf("decoded = %q, want %q", decoded, plaintext)
	}
}

func TestEncryptedSQLSecretStoreRejectsInvalidConfiguration(t *testing.T) {
	if _, err := NewEncryptedSQLSecretStore(nil, bytes.Repeat([]byte{0x42}, 32)); err == nil {
		t.Fatal("NewEncryptedSQLSecretStore() accepted nil database")
	}
}

func TestEncryptedSQLSecretStoreRejectsNonPositiveOperationTimeout(t *testing.T) {
	database, _, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	if _, err := NewEncryptedSQLSecretStoreWithTimeout(database, bytes.Repeat([]byte{0x42}, 32), 0); err == nil {
		t.Fatal("NewEncryptedSQLSecretStoreWithTimeout() error = nil, want invalid timeout")
	}
}

func TestEncryptedSQLSecretStorePutHonorsOperationTimeout(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	store, err := NewEncryptedSQLSecretStoreWithTimeout(database, bytes.Repeat([]byte{0x42}, 32), 10*time.Millisecond)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO model_account_secrets")).
		WithArgs("model-account/timeout", sqlmock.AnyArg()).
		WillDelayFor(50 * time.Millisecond).
		WillReturnResult(sqlmock.NewResult(1, 1))
	if err := store.Put(context.Background(), "model-account/timeout", "secret-value"); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("Put() error = %v, want context deadline exceeded", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestEncryptedSQLSecretStorePutTxUsesCallerTransaction(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	store, err := NewEncryptedSQLSecretStore(database, bytes.Repeat([]byte{0x42}, 32))
	if err != nil {
		t.Fatalf("NewEncryptedSQLSecretStore() error = %v", err)
	}
	mock.ExpectBegin()
	mock.ExpectExec(regexp.QuoteMeta("INSERT INTO model_account_secrets")).WithArgs("model-account/tx", sqlmock.AnyArg()).WillReturnResult(sqlmock.NewResult(1, 1))
	mock.ExpectCommit()
	tx, err := database.Begin()
	if err != nil {
		t.Fatalf("Begin() error = %v", err)
	}
	if err := store.PutTx(context.Background(), tx, "model-account/tx", "secret-value"); err != nil {
		t.Fatalf("PutTx() error = %v", err)
	}
	if err := tx.Commit(); err != nil {
		t.Fatalf("Commit() error = %v", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestEncryptedSQLSecretStorePutTxRejectsNilTransaction(t *testing.T) {
	database, _, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	store, err := NewEncryptedSQLSecretStore(database, bytes.Repeat([]byte{0x42}, 32))
	if err != nil {
		t.Fatalf("NewEncryptedSQLSecretStore() error = %v", err)
	}
	if err := store.PutTx(context.Background(), (*sql.Tx)(nil), "model-account/tx", "secret-value"); err == nil {
		t.Fatal("PutTx() accepted nil transaction")
	}
}

func TestEncryptedSQLSecretStoreCleanupStagedSecretsUsesProtectedReferencesAndBatch(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	store, err := NewEncryptedSQLSecretStore(database, bytes.Repeat([]byte{0x42}, 32))
	if err != nil {
		t.Fatalf("NewEncryptedSQLSecretStore() error = %v", err)
	}
	cutoff := time.Date(2026, 8, 21, 10, 0, 0, 0, time.UTC)
	protected := []string{"model-account/account-1/rotation_live"}
	mock.ExpectExec(regexp.QuoteMeta(cleanupStagedSecretsSQL)).
		WithArgs(cutoff, pq.Array(protected), 13).
		WillReturnResult(sqlmock.NewResult(0, 2))
	deleted, err := store.CleanupStagedSecrets(context.Background(), RetentionCleanupRequest{Cutoff: cutoff, BatchSize: 13}, protected)
	if err != nil {
		t.Fatalf("CleanupStagedSecrets() error = %v", err)
	}
	if deleted != 2 {
		t.Fatalf("deleted = %d, want 2", deleted)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestEncryptedSQLSecretStoreCleanupSecretReferencesUsesBoundedReturningDelete(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	store, err := NewEncryptedSQLSecretStore(database, bytes.Repeat([]byte{0x42}, 32))
	if err != nil {
		t.Fatalf("NewEncryptedSQLSecretStore() error = %v", err)
	}
	cutoff := time.Date(2026, 8, 21, 10, 0, 0, 0, time.UTC)
	references := []string{"model-account/account-1", "model-account/account-2"}
	protected := []string{"model-account/account-2"}
	mock.ExpectQuery(regexp.QuoteMeta(cleanupSecretReferencesSQL)).
		WithArgs(pq.Array(references), cutoff, pq.Array(protected), 2).
		WillReturnRows(sqlmock.NewRows([]string{"secret_ref"}).AddRow("model-account/account-1"))
	deleted, err := store.CleanupSecretReferences(context.Background(), RetentionCleanupRequest{Cutoff: cutoff, BatchSize: 2}, references, protected)
	if err != nil {
		t.Fatalf("CleanupSecretReferences() error = %v", err)
	}
	if len(deleted) != 1 || deleted[0] != "model-account/account-1" {
		t.Fatalf("deleted references = %v, want account-1 only", deleted)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
