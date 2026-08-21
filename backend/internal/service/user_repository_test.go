package service

import (
	"context"
	"testing"
	"time"

	"golang.org/x/crypto/bcrypt"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

type normalizedUserWriterRepository struct {
	*store.MemoryStore
	called         bool
	scope          string
	key            string
	fingerprint    string
	record         store.UserCreateRecord
	updateCalled   bool
	updateRecord   store.UserUpdateRecord
	resetCalled    bool
	resetUserID    string
	resetHash      []byte
	disableCalled  bool
	disableUserID  string
	credentialUser controlplane.UserSummary
	credentialHash []byte
	credentialErr  error
	adminEnsureErr error
	adminReadyErr  error
	adminChange    controlplane.UserSummary
	adminChangeErr error
}

func (r *normalizedUserWriterRepository) UsesNormalizedReadSource() bool { return true }

func (r *normalizedUserWriterRepository) CreateUser(_ context.Context, scope, key, fingerprint string, record store.UserCreateRecord) (controlplane.UserSummary, error) {
	r.called = true
	r.scope, r.key, r.fingerprint, r.record = scope, key, fingerprint, record
	return controlplane.UserSummary{ID: "usr_direct", Username: record.Username, Role: record.Role, Status: controlplane.UserStatusActive, CreatedAt: record.CreatedAt.UTC().Format(time.RFC3339)}, nil
}

func (r *normalizedUserWriterRepository) UpdateUser(_ context.Context, _, _, _ string, record store.UserUpdateRecord) (controlplane.UserSummary, error) {
	r.updateCalled = true
	r.updateRecord = record
	return controlplane.UserSummary{ID: record.UserID, Username: "alice", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: time.Now().UTC().Format(time.RFC3339)}, nil
}

func (r *normalizedUserWriterRepository) ResetUserPassword(_ context.Context, _, _, _, userID string, passwordHash []byte) (controlplane.UserSummary, error) {
	r.resetCalled = true
	r.resetUserID = userID
	r.resetHash = append([]byte(nil), passwordHash...)
	return controlplane.UserSummary{ID: userID, Username: "alice", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive}, nil
}

func (r *normalizedUserWriterRepository) DisableUser(_ context.Context, _, _, _, userID string) (controlplane.UserSummary, error) {
	r.disableCalled = true
	r.disableUserID = userID
	return controlplane.UserSummary{ID: userID, Username: "alice", Role: controlplane.RoleUser, Status: controlplane.UserStatusDisabled}, nil
}

func (r *normalizedUserWriterRepository) GetUserCredential(_ context.Context, _ string) (controlplane.UserSummary, []byte, error) {
	return r.credentialUser, append([]byte(nil), r.credentialHash...), r.credentialErr
}

func (r *normalizedUserWriterRepository) GetUserByID(_ context.Context, _ string) (controlplane.UserSummary, error) {
	return r.credentialUser, r.credentialErr
}

func (r *normalizedUserWriterRepository) EnsureConfiguredAdmin(_ context.Context, _ string, _ []byte) error {
	return r.adminEnsureErr
}

func (r *normalizedUserWriterRepository) CheckAdminReady(_ context.Context) error {
	return r.adminReadyErr
}

func (r *normalizedUserWriterRepository) ChangeLocalAdminPassword(_ context.Context, _, _, _ string, _ []byte) (controlplane.UserSummary, error) {
	return r.adminChange, r.adminChangeErr
}

func TestNormalizedAdminCredentialOperationsUseRepositoryBoundary(t *testing.T) {
	repository := &normalizedUserWriterRepository{
		MemoryStore: store.NewMemoryStore(time.Now),
		adminChange: controlplane.UserSummary{ID: "usr_local_admin", Username: "admin", Role: controlplane.RoleAdmin, Status: controlplane.UserStatusActive},
	}
	svc := NewControlPlaneWithRepository(repository)
	if err := svc.EnsureConfiguredAdmin(context.Background(), "admin", "first-password"); err != nil {
		t.Fatalf("EnsureConfiguredAdmin() error = %v", err)
	}
	if err := svc.CheckReady(context.Background()); err != nil {
		t.Fatalf("CheckReady() error = %v", err)
	}
	user, err := svc.ChangeLocalAdminPassword(context.Background(), "rotate-admin", controlplane.ChangeLocalAdminPasswordInput{Password: "rotated-password"})
	if err != nil {
		t.Fatalf("ChangeLocalAdminPassword() error = %v", err)
	}
	if user.ID != "usr_local_admin" {
		t.Fatalf("changed admin = %+v", user)
	}
}

func TestAuthenticateUserUsesNormalizedCredentialReader(t *testing.T) {
	hash, err := bcrypt.GenerateFromPassword([]byte("correct-password"), bcrypt.MinCost)
	if err != nil {
		t.Fatalf("bcrypt.GenerateFromPassword() error = %v", err)
	}
	repository := &normalizedUserWriterRepository{
		MemoryStore:    store.NewMemoryStore(time.Now),
		credentialUser: controlplane.UserSummary{ID: "usr_normalized", Username: "normalized", Role: controlplane.RoleUser, Status: controlplane.UserStatusActive, CreatedAt: "2026-08-21T12:00:00Z"},
		credentialHash: hash,
	}
	svc := NewControlPlaneWithRepository(repository)
	actor, user, err := svc.AuthenticateUser(context.Background(), "normalized", "correct-password")
	if err != nil {
		t.Fatalf("AuthenticateUser() error = %v", err)
	}
	if actor.UserID != user.ID || actor.Role != user.Role || user.ID != "usr_normalized" {
		t.Fatalf("normalized authentication = actor:%+v user:%+v", actor, user)
	}
	if _, _, err := svc.AuthenticateUser(context.Background(), "normalized", "wrong-password"); !controlplane.IsErrorCode(err, controlplane.ErrUnauthenticated.Code) {
		t.Fatalf("wrong password error = %v, want unauthenticated", err)
	}
	repository.credentialUser.Status = controlplane.UserStatusDisabled
	if _, _, err := svc.AuthenticateUser(context.Background(), "normalized", "correct-password"); !controlplane.IsErrorCode(err, controlplane.ErrUserDisabled.Code) {
		t.Fatalf("disabled user error = %v, want user disabled", err)
	}
	user, err = svc.GetUser(context.Background(), "usr_normalized")
	if err != nil || user.ID != "usr_normalized" {
		t.Fatalf("normalized GetUser() = user:%+v err:%v", user, err)
	}
}

func TestUserMutationsUseNormalizedUserRepositoryWriter(t *testing.T) {
	repository := &normalizedUserWriterRepository{MemoryStore: store.NewMemoryStore(time.Now)}
	svc := NewControlPlaneWithRepository(repository)
	username := "alice-renamed"
	role := controlplane.RoleAdmin
	user, err := svc.UpdateUser(context.Background(), "key-update", "usr_1", controlplane.UpdateUserInput{Username: &username, Role: &role})
	if err != nil {
		t.Fatalf("UpdateUser() error = %v", err)
	}
	if !repository.updateCalled || user.ID != "usr_1" || repository.updateRecord.UserID != "usr_1" || repository.updateRecord.Username == nil || *repository.updateRecord.Username != username || repository.updateRecord.Role == nil || *repository.updateRecord.Role != role {
		t.Fatalf("normalized update = called %t user %+v record %+v", repository.updateCalled, user, repository.updateRecord)
	}
	user, err = svc.ResetUserPassword(context.Background(), "key-reset", "usr_1", controlplane.ResetUserPasswordInput{Password: "correct horse battery"})
	if err != nil {
		t.Fatalf("ResetUserPassword() error = %v", err)
	}
	if !repository.resetCalled || user.ID != "usr_1" || repository.resetUserID != "usr_1" || len(repository.resetHash) == 0 {
		t.Fatalf("normalized reset = called %t user %+v user_id %q hash_len %d", repository.resetCalled, user, repository.resetUserID, len(repository.resetHash))
	}
	user, err = svc.DisableUser(context.Background(), "key-disable", "usr_1")
	if err != nil {
		t.Fatalf("DisableUser() error = %v", err)
	}
	if !repository.disableCalled || user.ID != "usr_1" || repository.disableUserID != "usr_1" {
		t.Fatalf("normalized disable = called %t user %+v user_id %q", repository.disableCalled, user, repository.disableUserID)
	}
}

func TestCreateUserUsesNormalizedUserRepositoryWriter(t *testing.T) {
	now := time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
	repository := &normalizedUserWriterRepository{MemoryStore: store.NewMemoryStore(func() time.Time { return now })}
	svc := NewControlPlaneWithRepository(repository)
	user, err := svc.CreateUser(context.Background(), "key-direct", controlplane.CreateUserInput{Username: "alice", Password: "correct horse battery", Role: controlplane.RoleUser})
	if err != nil {
		t.Fatalf("CreateUser() error = %v", err)
	}
	if !repository.called || user.ID != "usr_direct" {
		t.Fatalf("direct repository call = %t, user = %+v", repository.called, user)
	}
	if repository.scope != "control-plane-state" || repository.key != "create-user:key-direct" || repository.fingerprint == "" || repository.record.Username != "alice" || len(repository.record.PasswordHash) == 0 {
		t.Fatalf("normalized writer arguments = scope %q key %q fingerprint %q record %+v", repository.scope, repository.key, repository.fingerprint, repository.record)
	}
}
