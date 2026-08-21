package store

import (
	"context"
	"database/sql"
	"errors"
	"strings"
	"sync"
)

var ErrSecretNotFound = errors.New("secret not found")

// SecretStore 是模型密钥的独立存储边界。业务 State 只保存 SecretRef，不能保存明文密钥。
type SecretStore interface {
	Ping(ctx context.Context) error
	Put(ctx context.Context, reference, value string) error
	Get(ctx context.Context, reference string) (string, error)
	Delete(ctx context.Context, reference string) error
}

// TransactionalSecretWriter is implemented by durable SecretStores that can
// write a ciphertext through a caller-owned SQL transaction. It is the only
// safe boundary for normalized account creation: the account row and its
// secret must commit or roll back together.
type TransactionalSecretWriter interface {
	PutTx(ctx context.Context, tx *sql.Tx, reference, value string) error
}

var ErrTransactionalSecretStoreRequired = errors.New("transactional secret store required")

// MemorySecretStore 仅用于本地开发和测试；进程退出后密钥丢失。
type MemorySecretStore struct {
	mu     sync.Mutex
	values map[string]string
}

func NewMemorySecretStore() *MemorySecretStore {
	return &MemorySecretStore{values: map[string]string{}}
}

func (s *MemorySecretStore) Ping(ctx context.Context) error {
	return ctx.Err()
}

func (s *MemorySecretStore) Put(ctx context.Context, reference, value string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	reference = strings.TrimSpace(reference)
	if reference == "" || strings.TrimSpace(value) == "" {
		return errors.New("secret reference and value must not be empty")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	s.values[reference] = value
	return nil
}

func (s *MemorySecretStore) Get(ctx context.Context, reference string) (string, error) {
	if err := ctx.Err(); err != nil {
		return "", err
	}
	reference = strings.TrimSpace(reference)
	s.mu.Lock()
	defer s.mu.Unlock()
	value, ok := s.values[reference]
	if !ok {
		return "", ErrSecretNotFound
	}
	return value, nil
}

func (s *MemorySecretStore) Delete(ctx context.Context, reference string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	reference = strings.TrimSpace(reference)
	s.mu.Lock()
	defer s.mu.Unlock()
	if _, ok := s.values[reference]; !ok {
		return ErrSecretNotFound
	}
	delete(s.values, reference)
	return nil
}

func (s *MemorySecretStore) CleanupSecretReferences(ctx context.Context, request RetentionCleanupRequest, references, protectedReferences []string) ([]string, error) {
	if err := request.Validate(); err != nil {
		return nil, err
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	protected := make(map[string]struct{}, len(protectedReferences))
	for _, reference := range protectedReferences {
		protected[strings.TrimSpace(reference)] = struct{}{}
	}
	candidates := normalizeSecretReferences(references, request.BatchSize)
	deleted := make([]string, 0, len(candidates))
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, reference := range candidates {
		if _, isProtected := protected[reference]; isProtected {
			continue
		}
		if _, exists := s.values[reference]; !exists {
			continue
		}
		delete(s.values, reference)
		deleted = append(deleted, reference)
	}
	return deleted, nil
}
