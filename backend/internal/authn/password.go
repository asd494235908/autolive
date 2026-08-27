package authn

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/base64"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"unicode/utf8"

	"golang.org/x/crypto/argon2"
	"golang.org/x/crypto/bcrypt"
)

const (
	MinPasswordCharacters = 15
	MaxPasswordCharacters = 128
	MaxPasswordBytes      = 256

	argonMemory      = 19 * 1024
	argonIterations  = 2
	argonParallelism = 1
	argonSaltLength  = 16
	argonKeyLength   = 32
	maxArgonMemory   = 64 * 1024
	maxArgonTime     = 5
	maxArgonParallel = 4
)

var (
	ErrInvalidPasswordHash = errors.New("invalid password hash")
	dummySalt              = []byte("autolive-dummy!!")
	dummyDigest            = argon2.IDKey([]byte("dummy-password-never-used"), dummySalt, argonIterations, argonMemory, argonParallelism, argonKeyLength)
	processPasswordSlots   = make(chan struct{}, 4)
)

func ValidNewPassword(password string) bool {
	if !utf8.ValidString(password) || len(password) > MaxPasswordBytes {
		return false
	}
	characters := utf8.RuneCountInString(password)
	return characters >= MinPasswordCharacters && characters <= MaxPasswordCharacters
}

type PasswordManager struct {
	semaphore chan struct{}
}

func NewPasswordManager() *PasswordManager {
	return &PasswordManager{semaphore: processPasswordSlots}
}

func (m *PasswordManager) Hash(ctx context.Context, password string) ([]byte, error) {
	if err := m.acquire(ctx); err != nil {
		return nil, err
	}
	defer m.release()
	salt := make([]byte, argonSaltLength)
	if _, err := rand.Read(salt); err != nil {
		return nil, fmt.Errorf("generate password salt: %w", err)
	}
	digest := argon2.IDKey([]byte(password), salt, argonIterations, argonMemory, argonParallelism, argonKeyLength)
	return []byte(fmt.Sprintf("$argon2id$v=%d$m=%d,t=%d,p=%d$%s$%s",
		argon2.Version, argonMemory, argonIterations, argonParallelism,
		base64.RawStdEncoding.EncodeToString(salt), base64.RawStdEncoding.EncodeToString(digest))), nil
}

func (m *PasswordManager) Verify(ctx context.Context, encoded []byte, password string) (valid, needsUpgrade bool, err error) {
	if isBcryptHash(encoded) {
		err = bcrypt.CompareHashAndPassword(encoded, []byte(password))
		return err == nil, err == nil, nil
	}
	parameters, salt, expected, err := parseArgon2id(encoded)
	if err != nil {
		return false, false, err
	}
	if err := m.acquire(ctx); err != nil {
		return false, false, err
	}
	defer m.release()
	actual := argon2.IDKey([]byte(password), salt, parameters.iterations, parameters.memory, parameters.parallelism, uint32(len(expected)))
	return subtle.ConstantTimeCompare(actual, expected) == 1, parameters.needsUpgrade(), nil
}

// DummyVerify burns the same Argon2id work as a real login when no usable
// credential exists. Its result is intentionally discarded by callers.
func (m *PasswordManager) DummyVerify(ctx context.Context, password string) error {
	if err := m.acquire(ctx); err != nil {
		return err
	}
	defer m.release()
	actual := argon2.IDKey([]byte(password), dummySalt, argonIterations, argonMemory, argonParallelism, argonKeyLength)
	_ = subtle.ConstantTimeCompare(actual, dummyDigest)
	return nil
}

func (m *PasswordManager) acquire(ctx context.Context) error {
	if ctx == nil {
		return errors.New("password operation context is required")
	}
	select {
	case m.semaphore <- struct{}{}:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (m *PasswordManager) release() { <-m.semaphore }

func isBcryptHash(encoded []byte) bool {
	return len(encoded) >= 4 && encoded[0] == '$' && encoded[1] == '2' && (encoded[2] == 'a' || encoded[2] == 'b' || encoded[2] == 'y') && encoded[3] == '$'
}

type argon2Parameters struct {
	memory      uint32
	iterations  uint32
	parallelism uint8
	saltLength  int
	keyLength   int
}

func (p argon2Parameters) needsUpgrade() bool {
	return p.memory != argonMemory || p.iterations != argonIterations || p.parallelism != argonParallelism || p.saltLength != argonSaltLength || p.keyLength != argonKeyLength
}

func parseArgon2id(encoded []byte) (argon2Parameters, []byte, []byte, error) {
	parts := strings.Split(string(encoded), "$")
	if len(parts) != 6 || parts[0] != "" || parts[1] != "argon2id" || parts[2] != "v=19" {
		return argon2Parameters{}, nil, nil, ErrInvalidPasswordHash
	}
	var values = map[string]uint64{}
	for _, pair := range strings.Split(parts[3], ",") {
		key, raw, ok := strings.Cut(pair, "=")
		if !ok {
			return argon2Parameters{}, nil, nil, ErrInvalidPasswordHash
		}
		if _, duplicate := values[key]; duplicate {
			return argon2Parameters{}, nil, nil, ErrInvalidPasswordHash
		}
		value, err := strconv.ParseUint(raw, 10, 32)
		if err != nil {
			return argon2Parameters{}, nil, nil, ErrInvalidPasswordHash
		}
		values[key] = value
	}
	if len(values) != 3 || values["m"] < 8 || values["m"] > maxArgonMemory || values["t"] < 1 || values["t"] > maxArgonTime || values["p"] < 1 || values["p"] > maxArgonParallel {
		return argon2Parameters{}, nil, nil, ErrInvalidPasswordHash
	}
	salt, err := base64.RawStdEncoding.Strict().DecodeString(parts[4])
	if err != nil || len(salt) < 8 || len(salt) > 64 {
		return argon2Parameters{}, nil, nil, ErrInvalidPasswordHash
	}
	digest, err := base64.RawStdEncoding.Strict().DecodeString(parts[5])
	if err != nil || len(digest) < 16 || len(digest) > 64 {
		return argon2Parameters{}, nil, nil, ErrInvalidPasswordHash
	}
	return argon2Parameters{memory: uint32(values["m"]), iterations: uint32(values["t"]), parallelism: uint8(values["p"]), saltLength: len(salt), keyLength: len(digest)}, salt, digest, nil
}
