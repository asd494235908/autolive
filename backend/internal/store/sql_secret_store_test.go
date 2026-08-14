package store

import (
	"bytes"
	"testing"
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
