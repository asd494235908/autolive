package authn

import (
	"context"
	"strings"
	"testing"

	"golang.org/x/crypto/bcrypt"
)

func TestPasswordManagerHashAndVerifyArgon2id(t *testing.T) {
	manager := NewPasswordManager()
	hash, err := manager.Hash(context.Background(), "a-secure-password")
	if err != nil {
		t.Fatalf("Hash() error = %v", err)
	}
	if !strings.HasPrefix(string(hash), "$argon2id$v=19$m=19456,t=2,p=1$") {
		t.Fatalf("Hash() = %q, want OWASP Argon2id PHC parameters", hash)
	}
	valid, upgrade, err := manager.Verify(context.Background(), hash, "a-secure-password")
	if err != nil || !valid || upgrade {
		t.Fatalf("Verify() = valid %v upgrade %v error %v", valid, upgrade, err)
	}
	valid, _, err = manager.Verify(context.Background(), hash, "wrong-password")
	if err != nil || valid {
		t.Fatalf("Verify(wrong) = valid %v error %v", valid, err)
	}
}

func TestPasswordManagerAcceptsBcryptAndRequestsUpgrade(t *testing.T) {
	legacy, err := bcrypt.GenerateFromPassword([]byte("legacy-password"), bcrypt.MinCost)
	if err != nil {
		t.Fatal(err)
	}
	valid, upgrade, err := NewPasswordManager().Verify(context.Background(), legacy, "legacy-password")
	if err != nil || !valid || !upgrade {
		t.Fatalf("Verify(bcrypt) = valid %v upgrade %v error %v", valid, upgrade, err)
	}
}

func TestPasswordManagerRejectsMalformedPHC(t *testing.T) {
	if _, _, err := NewPasswordManager().Verify(context.Background(), []byte("$argon2id$broken"), "password"); err == nil {
		t.Fatal("Verify() error = nil, want malformed hash error")
	}
	oversized := []byte("$argon2id$v=19$m=65537,t=2,p=1$MTIzNDU2Nzg5MDEyMzQ1Ng$MTIzNDU2Nzg5MDEyMzQ1Njc4OTAxMjM0NTY")
	if _, _, err := NewPasswordManager().Verify(context.Background(), oversized, "password"); err == nil {
		t.Fatal("Verify() error = nil, want oversized Argon2 cost error")
	}
}

func TestPasswordManagerRequestsUpgradeForShortSaltOrDigest(t *testing.T) {
	manager := NewPasswordManager()
	for _, encoded := range []string{
		"$argon2id$v=19$m=19456,t=2,p=1$MTIzNDU2Nzg$MTIzNDU2Nzg5MDEyMzQ1Ng",
		"$argon2id$v=19$m=19456,t=2,p=1$MTIzNDU2Nzg5MDEyMzQ1Ng$MTIzNDU2Nzg5MDEyMzQ1Ng",
	} {
		_, upgrade, err := manager.Verify(context.Background(), []byte(encoded), "password")
		if err != nil || !upgrade {
			t.Fatalf("Verify(%q) upgrade = %v, error %v", encoded, upgrade, err)
		}
	}
	if _, _, err := manager.Verify(context.Background(), []byte("$argon2id$v=19$m=19456,m=19456,t=2,p=1$MTIzNDU2Nzg5MDEyMzQ1Ng$MTIzNDU2Nzg5MDEyMzQ1Ng"), "password"); err == nil {
		t.Fatal("duplicate PHC parameter accepted")
	}
}

func TestValidNewPasswordUsesUnicodeCharactersAndByteLimit(t *testing.T) {
	if !ValidNewPassword(strings.Repeat("密", 15)) {
		t.Fatal("15 Unicode characters rejected")
	}
	if ValidNewPassword(strings.Repeat("密", 86)) {
		t.Fatal("password exceeding 256 bytes accepted")
	}
	if ValidNewPassword(strings.Repeat("a", 129)) || ValidNewPassword(strings.Repeat("a", 14)) {
		t.Fatal("password character bounds not enforced")
	}
}
