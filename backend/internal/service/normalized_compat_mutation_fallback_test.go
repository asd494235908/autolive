package service

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestNormalizedCompatibilityMutationsFailClosedWithoutDomainRepositories(t *testing.T) {
	repository := &normalizedCompatibilityMutationRepository{}
	service := NewControlPlaneWithRepository(repository)
	ctx := context.Background()

	tests := []struct {
		name string
		want error
		call func() error
	}{
		{name: "ensure_local_admin", want: store.ErrNormalizedLocalAdminBootstrapRequired, call: func() error {
			return service.EnsureLocalAdmin(ctx, "admin")
		}},
		{name: "record_audit", want: store.ErrNormalizedAuditRepositoryRequired, call: func() error {
			return service.RecordAudit(ctx, controlplane.AuditLogInput{
				Action: "user.update", TargetType: "user", Outcome: "success", StatusCode: 200,
			})
		}},
		{name: "update_user_authorization", want: store.ErrNormalizedUserAuthorizationRepositoryRequired, call: func() error {
			_, err := service.UpdateUserAuthorization(ctx, "policy-key", "usr-1", controlplane.UpdateUserAuthorizationInput{
				AllowedModels: []string{"openai/rewrite"}, DailyTokenLimit: 100,
			})
			return err
		}},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if err := test.call(); !errors.Is(err, test.want) {
				t.Fatalf("error = %v, want %v", err, test.want)
			}
		})
	}
	if repository.runCalls != 0 {
		t.Fatalf("normalized compatibility mutations used StateOperation %d times", repository.runCalls)
	}
}

type normalizedCompatibilityMutationRepository struct {
	runCalls int
}

func (*normalizedCompatibilityMutationRepository) Now() time.Time {
	return time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
}

func (r *normalizedCompatibilityMutationRepository) Run(context.Context, store.StateOperation) error {
	r.runCalls++
	return errors.New("normalized compatibility mutation must not use StateOperation")
}

func (*normalizedCompatibilityMutationRepository) UsesNormalizedReadSource() bool { return true }

func TestNormalizedSecretRotationPreparationDoesNotUseStateOperation(t *testing.T) {
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"data":[{"id":"rewrite-model"}]}`))
	}))
	defer provider.Close()
	client, resolver := modelPoolTestHTTPClient(t, provider)
	repository := &normalizedRotationPreparationRepository{account: controlplane.ModelPoolAccountSummary{
		ID: "mpa_1", Provider: "openai-compatible", Model: "rewrite-model", BaseURL: "http://model.test/v1",
		Status: controlplane.ModelAccountStatusActive, SecretRef: "model-account/mpa_1",
	}}
	service := newControlPlaneWithRepositoryAndSecretStore(repository, client, store.NewMemorySecretStore(), resolver)
	rotated, err := service.RotateModelPoolAccountSecret(context.Background(), "rotation-key", "mpa_1", controlplane.RotateModelPoolAccountSecretInput{
		APIKey: "new-secret", TimeoutSeconds: 5,
	})
	if err != nil {
		t.Fatalf("RotateModelPoolAccountSecret() error = %v", err)
	}
	if rotated.ID != "mpa_1" || repository.record.ExpectedSecretRef != "model-account/mpa_1" {
		t.Fatalf("rotation result = %+v, record = %+v", rotated, repository.record)
	}
	if repository.runCalls != 0 {
		t.Fatalf("normalized secret rotation used StateOperation %d times", repository.runCalls)
	}
}

func TestNormalizedSecretRotationFailsClosedWithoutPreparation(t *testing.T) {
	repository := &normalizedCompatibilityMutationRepository{}
	service := NewControlPlaneWithRepository(repository)
	_, err := service.RotateModelPoolAccountSecret(context.Background(), "rotation-key", "mpa_1", controlplane.RotateModelPoolAccountSecretInput{
		APIKey: "new-secret", TimeoutSeconds: 5,
	})
	if !errors.Is(err, store.ErrNormalizedModelPoolSecretRotationPreparerRequired) {
		t.Fatalf("RotateModelPoolAccountSecret() error = %v, want preparation requirement", err)
	}
	if repository.runCalls != 0 {
		t.Fatalf("normalized rotation fallback used StateOperation %d times", repository.runCalls)
	}
}

func TestNormalizedSecretRotationFailsClosedWithoutTransactionalRotator(t *testing.T) {
	repository := &normalizedRotationPreparerOnlyRepository{}
	service := NewControlPlaneWithRepository(repository)
	_, err := service.RotateModelPoolAccountSecret(context.Background(), "rotation-key", "mpa_1", controlplane.RotateModelPoolAccountSecretInput{
		APIKey: "new-secret", TimeoutSeconds: 5,
	})
	if !errors.Is(err, store.ErrNormalizedModelPoolSecretRotatorRequired) {
		t.Fatalf("RotateModelPoolAccountSecret() error = %v, want rotator requirement", err)
	}
	if repository.runCalls != 0 {
		t.Fatalf("normalized rotation fallback used StateOperation %d times", repository.runCalls)
	}
}

type normalizedRotationPreparationRepository struct {
	account  controlplane.ModelPoolAccountSummary
	record   store.ModelPoolSecretRotationRecord
	runCalls int
}

func (r *normalizedRotationPreparationRepository) Now() time.Time {
	return time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
}

func (r *normalizedRotationPreparationRepository) Run(context.Context, store.StateOperation) error {
	r.runCalls++
	return errors.New("normalized secret rotation must not use StateOperation")
}

func (*normalizedRotationPreparationRepository) UsesNormalizedReadSource() bool { return true }

func (r *normalizedRotationPreparationRepository) PrepareModelPoolAccountSecretRotation(context.Context, string, string, string, string) (store.ModelPoolSecretRotationPreparation, error) {
	return store.ModelPoolSecretRotationPreparation{Account: r.account}, nil
}

func (r *normalizedRotationPreparationRepository) RotateModelPoolAccountSecret(_ context.Context, record store.ModelPoolSecretRotationRecord) (controlplane.ModelPoolAccountSummary, error) {
	r.record = record
	return controlplane.ModelPoolAccountSummary{ID: record.AccountID, SecretConfigured: true, SecretRef: record.ExpectedSecretRef + "/rotation_test"}, nil
}

type normalizedRotationPreparerOnlyRepository struct {
	runCalls int
}

func (*normalizedRotationPreparerOnlyRepository) Now() time.Time {
	return time.Date(2026, 8, 21, 12, 0, 0, 0, time.UTC)
}

func (r *normalizedRotationPreparerOnlyRepository) Run(context.Context, store.StateOperation) error {
	r.runCalls++
	return errors.New("normalized secret rotation must not use StateOperation")
}

func (*normalizedRotationPreparerOnlyRepository) UsesNormalizedReadSource() bool { return true }

func (*normalizedRotationPreparerOnlyRepository) PrepareModelPoolAccountSecretRotation(context.Context, string, string, string, string) (store.ModelPoolSecretRotationPreparation, error) {
	return store.ModelPoolSecretRotationPreparation{}, nil
}
