package httpapi

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestActivateBindsBeforeBusinessMutationWhenSessionStoreFails(t *testing.T) {
	testNow := time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return testNow })
	sessions := newTestSessionStore()
	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStore(
		"test", nil, AuthConfig{Username: "admin", Password: "password"}, repository, store.NewMemorySecretStore(), sessions,
	)
	token := loginForTest(t, handler)
	adminCode := doJSON(t, handler, http.MethodPost, "/api/v1/admin/activation-codes", map[string]any{
		"user_id": "usr_local_admin", "expires_at": testActivationExpiresAt(), "max_devices": 1,
	}, token, "activation-prepare-1")
	if adminCode.Code != http.StatusCreated {
		t.Fatalf("create activation code status = %d; body=%s", adminCode.Code, adminCode.Body.String())
	}
	var codeEnvelope activationCodeEnvelope
	decodeJSON(t, adminCode.Body.Bytes(), &codeEnvelope)
	if codeEnvelope.ActivationCode.PlainCode == nil {
		t.Fatal("activation code response did not contain one-time plaintext")
	}

	sessions.mu.Lock()
	sessions.updateErr = errors.New("session store unavailable")
	sessions.mu.Unlock()
	response := doJSON(t, handler, http.MethodPost, "/api/v1/client/activate", controlplane.ActivateDeviceInput{
		Device: controlplane.DeviceRegistration{Product: controlplane.ProductAutoLive, DeviceID: "dev_bindfail1", DeviceName: "Test", Platform: "windows", AppVersion: "1.0.0"},
	}, token, "activation-bind-failure")
	if response.Code != http.StatusServiceUnavailable {
		t.Fatalf("activation with unavailable session store status = %d; body=%s", response.Code, response.Body.String())
	}
	var stateSnapshot *store.State
	if err := repository.Run(context.Background(), func(state *store.State) error {
		stateSnapshot = state
		return nil
	}); err != nil {
		t.Fatalf("inspect state: %v", err)
	}
	if _, ok := stateSnapshot.Devices["dev_bindfail1"]; ok {
		t.Fatal("device was persisted despite session bind failure")
	}
	if record := stateSnapshot.ActivationCodes[codeEnvelope.ActivationCode.ID]; record.ActivationCode.Status != controlplane.ActivationCodeStatusActive {
		t.Fatalf("activation code status = %q, want active", record.ActivationCode.Status)
	}
}

func TestActivateCompensatesNewSessionBindingWhenBusinessFails(t *testing.T) {
	testNow := time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return testNow })
	sessions := newTestSessionStore()
	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStore(
		"test", nil, AuthConfig{Username: "admin", Password: "password"}, repository, store.NewMemorySecretStore(), sessions,
	)
	token := loginForTest(t, handler)
	response := doJSON(t, handler, http.MethodPost, "/api/v1/client/activate", controlplane.ActivateDeviceInput{
		Device: controlplane.DeviceRegistration{Product: controlplane.ProductAutoLive, DeviceID: "dev_compens1", DeviceName: "Test", Platform: "windows", AppVersion: "1.0.0"},
	}, token, "activation-compensation")
	if response.Code != http.StatusForbidden {
		t.Fatalf("activation business failure status = %d; body=%s", response.Code, response.Body.String())
	}

	var sessionDeviceID string
	sessions.mu.Lock()
	for _, session := range sessions.byAccess {
		sessionDeviceID = session.DeviceID
		break
	}
	sessions.mu.Unlock()
	if sessionDeviceID != "" {
		t.Fatalf("compensated session device id = %q, want empty", sessionDeviceID)
	}
	var body map[string]any
	if err := json.Unmarshal(response.Body.Bytes(), &body); err != nil {
		t.Fatalf("decode error response: %v", err)
	}
	if body["code"] != "ACCOUNT_ACTIVATION_REQUIRED" {
		t.Fatalf("error code = %v, want ACCOUNT_ACTIVATION_REQUIRED", body["code"])
	}
}

func TestHeartbeatCompensatesNewSessionBindingWhenBusinessFails(t *testing.T) {
	testNow := time.Date(2026, 8, 13, 10, 0, 0, 0, time.UTC)
	repository := store.NewMemoryStore(func() time.Time { return testNow })
	sessions := newTestSessionStore()
	handler := NewRouterWithRepositoryAndSecretStoreAndSessionStore(
		"test", nil, AuthConfig{Username: "admin", Password: "password"}, repository, store.NewMemorySecretStore(), sessions,
	)
	token := loginForTest(t, handler)
	response := doJSON(t, handler, http.MethodPost, "/api/v1/client/heartbeat", controlplane.HeartbeatInput{
		Product: controlplane.ProductAutoLive, DeviceID: "dev_hb_fail1", SentAt: testNow,
	}, token, "heartbeat-compensation")
	if response.Code != http.StatusNotFound {
		t.Fatalf("heartbeat business failure status = %d; body=%s", response.Code, response.Body.String())
	}
	sessions.mu.Lock()
	defer sessions.mu.Unlock()
	for _, session := range sessions.byAccess {
		if session.DeviceID != "" {
			t.Fatalf("compensated heartbeat session device id = %q, want empty", session.DeviceID)
		}
	}
}
