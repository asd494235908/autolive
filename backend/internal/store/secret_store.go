package store

import (
	"context"
	"errors"
	"strings"
	"sync"
)

var ErrSecretNotFound = errors.New("secret not found")

// SecretStore 是模型密钥的独立存储边界。业务 State 只保存 SecretRef，不能保存明文密钥。
type SecretStore interface {
	Put(ctx context.Context, reference, value string) error
	Get(ctx context.Context, reference string) (string, error)
	Delete(ctx context.Context, reference string) error
}

// MemorySecretStore 仅用于本地开发和测试；进程退出后密钥丢失。
type MemorySecretStore struct {
	mu     sync.Mutex
	values map[string]string
}

func NewMemorySecretStore() *MemorySecretStore {
	return &MemorySecretStore{values: map[string]string{}}
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
