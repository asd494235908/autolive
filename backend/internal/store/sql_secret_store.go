package store

import (
	"context"
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"database/sql"
	"errors"
	"fmt"
	"strings"
)

// EncryptedSQLSecretStore 将密钥以 AES-256-GCM 密文保存到受控数据库表。
// 加密主密钥不进入数据库，必须由部署环境的 Secret Store 注入进程。
type EncryptedSQLSecretStore struct {
	db  *sql.DB
	key []byte
}

func NewEncryptedSQLSecretStore(db *sql.DB, key []byte) (*EncryptedSQLSecretStore, error) {
	if db == nil {
		return nil, errors.New("secret store database must not be nil")
	}
	if len(key) != 32 {
		return nil, errors.New("secret store key must be 32 bytes")
	}
	return &EncryptedSQLSecretStore{db: db, key: append([]byte(nil), key...)}, nil
}

func (s *EncryptedSQLSecretStore) Put(ctx context.Context, reference, value string) error {
	reference = strings.TrimSpace(reference)
	value = strings.TrimSpace(value)
	if reference == "" || value == "" {
		return errors.New("secret reference and value must not be empty")
	}
	ciphertext, err := encryptSecret(s.key, []byte(value))
	if err != nil {
		return fmt.Errorf("encrypt secret: %w", err)
	}
	_, err = s.db.ExecContext(ctx, `
		INSERT INTO model_account_secrets (secret_ref, ciphertext, updated_at)
		VALUES ($1, $2, CURRENT_TIMESTAMP)
		ON CONFLICT (secret_ref) DO UPDATE SET ciphertext = EXCLUDED.ciphertext, updated_at = CURRENT_TIMESTAMP
	`, reference, ciphertext)
	return err
}

func (s *EncryptedSQLSecretStore) Get(ctx context.Context, reference string) (string, error) {
	reference = strings.TrimSpace(reference)
	var ciphertext []byte
	err := s.db.QueryRowContext(ctx, `SELECT ciphertext FROM model_account_secrets WHERE secret_ref = $1`, reference).Scan(&ciphertext)
	if errors.Is(err, sql.ErrNoRows) {
		return "", ErrSecretNotFound
	}
	if err != nil {
		return "", err
	}
	plaintext, err := decryptSecret(s.key, ciphertext)
	if err != nil {
		return "", fmt.Errorf("decrypt secret: %w", err)
	}
	return string(plaintext), nil
}

func (s *EncryptedSQLSecretStore) Delete(ctx context.Context, reference string) error {
	result, err := s.db.ExecContext(ctx, `DELETE FROM model_account_secrets WHERE secret_ref = $1`, strings.TrimSpace(reference))
	if err != nil {
		return err
	}
	rows, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if rows == 0 {
		return ErrSecretNotFound
	}
	return nil
}

func encryptSecret(key, plaintext []byte) ([]byte, error) {
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, err
	}
	nonce := make([]byte, gcm.NonceSize())
	if _, err := rand.Read(nonce); err != nil {
		return nil, err
	}
	return gcm.Seal(nonce, nonce, plaintext, nil), nil
}

func decryptSecret(key, ciphertext []byte) ([]byte, error) {
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, err
	}
	if len(ciphertext) < gcm.NonceSize() {
		return nil, errors.New("secret ciphertext is shorter than nonce")
	}
	nonce, payload := ciphertext[:gcm.NonceSize()], ciphertext[gcm.NonceSize():]
	return gcm.Open(nil, nonce, payload, nil)
}
