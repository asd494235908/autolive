package service

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/url"
	"strings"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func checkContext(ctx context.Context) error {
	if ctx == nil {
		return controlplane.ErrInvalidRequest
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	default:
		return nil
	}
}

func validIdempotencyKey(value string) bool {
	trimmed := strings.TrimSpace(value)
	return trimmed == value && len(value) >= 8 && len(value) <= 128
}

func pageOffset(page, pageSize int) (int, error) {
	if page < 1 || pageSize < 1 || pageSize > 200 {
		return 0, controlplane.ErrInvalidRequest
	}
	maxInt := int(^uint(0) >> 1)
	if page-1 > maxInt/pageSize {
		return 0, controlplane.ErrInvalidRequest
	}
	return (page - 1) * pageSize, nil
}

func pageWindow(total, offset, limit int) (int, int) {
	if offset >= total {
		return total, total
	}
	end := offset + limit
	if end < offset || end > total {
		end = total
	}
	return offset, end
}

func nextID(state *store.State, prefix string) string {
	state.SequenceCounters[prefix]++
	return fmt.Sprintf("%s_%08d", prefix, state.SequenceCounters[prefix])
}

func fingerprintValue(value any) (string, error) {
	raw, err := json.Marshal(value)
	if err != nil {
		return "", controlplane.ErrInvalidRequest
	}
	sum := sha256.Sum256(raw)
	return hex.EncodeToString(sum[:]), nil
}

func secretDigest(value string) string {
	sum := sha256.Sum256([]byte(value))
	return hex.EncodeToString(sum[:])
}

func randomToken(prefix string, size int) (string, error) {
	buffer := make([]byte, size)
	if _, err := rand.Read(buffer); err != nil {
		return "", err
	}
	return prefix + hex.EncodeToString(buffer), nil
}

func validModelBaseURL(value string) bool {
	parsed, err := url.Parse(value)
	return err == nil &&
		(parsed.Scheme == "http" || parsed.Scheme == "https") &&
		parsed.Host != "" &&
		parsed.User == nil &&
		parsed.RawQuery == "" &&
		parsed.Fragment == "" &&
		parsed.Hostname() != "" &&
		!strings.Contains(parsed.Hostname(), "%")
}

func validateHeartbeatInput(input controlplane.HeartbeatInput) error {
	if !idPattern.MatchString(input.DeviceID) || input.SentAt.IsZero() {
		return controlplane.ErrInvalidRequest
	}
	if input.Product != "" && !input.Product.Valid() {
		return controlplane.ErrInvalidRequest
	}
	if input.Status.DiskFreeBytes < 0 || input.Status.MemoryTotalBytes < 0 || input.Status.MemoryAvailableBytes < 0 || (input.Status.MemoryTotalBytes > 0 && input.Status.MemoryAvailableBytes > input.Status.MemoryTotalBytes) || input.Status.CPULogicalCores < 0 || input.Status.CPULogicalCores > 4096 || len(input.Status.OSName) > 128 || len(input.Status.OSVersion) > 128 || len(input.Status.KernelVersion) > 128 || len(input.Status.CurrentMediaName) > 255 {
		return controlplane.ErrInvalidRequest
	}
	switch input.Status.PlaybackState {
	case "", "idle", "playing", "paused", "error":
		return nil
	default:
		return controlplane.ErrInvalidRequest
	}
}

func validateActivateDeviceInput(input controlplane.ActivateDeviceInput) error {
	if !idPattern.MatchString(input.Device.DeviceID) {
		return controlplane.ErrInvalidRequest
	}
	if strings.TrimSpace(input.Device.DeviceName) == "" || len(input.Device.DeviceName) > 128 || strings.TrimSpace(input.Device.Platform) == "" || len(input.Device.Platform) > 64 || strings.TrimSpace(input.Device.AppVersion) == "" || len(input.Device.AppVersion) > 64 || len(input.Device.OSVersion) > 128 {
		return controlplane.ErrInvalidRequest
	}
	if input.Device.Product != "" && !input.Device.Product.Valid() {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func validateTestModelPoolAccountInput(input *controlplane.TestModelPoolAccountInput) error {
	if input.TimeoutSeconds == 0 {
		input.TimeoutSeconds = 15
	}
	if input.TimeoutSeconds < 1 || input.TimeoutSeconds > 60 {
		return controlplane.ErrInvalidRequest
	}
	return nil
}

func validateRotateModelPoolAccountSecretInput(input *controlplane.RotateModelPoolAccountSecretInput) error {
	input.APIKey = strings.TrimSpace(input.APIKey)
	testInput := controlplane.TestModelPoolAccountInput{TimeoutSeconds: input.TimeoutSeconds}
	if len(input.APIKey) < 8 || len(input.APIKey) > 4096 || validateTestModelPoolAccountInput(&testInput) != nil {
		return controlplane.ErrInvalidRequest
	}
	input.TimeoutSeconds = testInput.TimeoutSeconds
	return nil
}

func validateDirectLLMCallRecordInput(input *controlplane.CreateDirectLLMCallRecordInput) error {
	input.ClientCallID = strings.TrimSpace(input.ClientCallID)
	input.LeaseID = strings.TrimSpace(input.LeaseID)
	input.Provider = strings.TrimSpace(input.Provider)
	input.Model = strings.TrimSpace(input.Model)
	input.Status = strings.TrimSpace(input.Status)
	input.UsageSource = strings.TrimSpace(input.UsageSource)
	input.ErrorCode = strings.TrimSpace(input.ErrorCode)
	input.FinishReason = strings.TrimSpace(input.FinishReason)
	if !idPattern.MatchString(input.ClientCallID) || !idPattern.MatchString(input.LeaseID) || len(input.Provider) == 0 || len(input.Provider) > 64 || len(input.Model) == 0 || len(input.Model) > 128 || input.InputTokens < 0 || input.OutputTokens < 0 || input.TotalTokens < 0 || input.LatencyMS < 0 || len(input.ErrorCode) > 128 || len(input.FinishReason) > 64 {
		return controlplane.ErrInvalidRequest
	}
	switch input.Status {
	case "succeeded", "failed", "timeout", "cancelled", "unknown":
	default:
		return controlplane.ErrInvalidRequest
	}
	if input.UsageSource != "client_reported" {
		return controlplane.ErrInvalidRequest
	}
	return nil
}
