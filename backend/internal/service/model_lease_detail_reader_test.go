package service

import (
	"context"
	"errors"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
)

func TestGetModelLeaseAdminDetailUsesNormalizedReaderWithoutStateFallback(t *testing.T) {
	reader := &normalizedLeaseDetailReader{detail: controlplane.ModelLeaseAdminDetail{ID: "lease-1", Status: controlplane.ModelLeaseStatusActive}}
	svc := NewControlPlaneWithRepository(reader)
	detail, err := svc.GetModelLeaseAdminDetail(context.Background(), "lease-1")
	if err != nil {
		t.Fatalf("GetModelLeaseAdminDetail() error = %v", err)
	}
	if detail.ID != "lease-1" || reader.runCalls != 0 || reader.leaseID != "lease-1" {
		t.Fatalf("detail=%+v runCalls=%d leaseID=%q", detail, reader.runCalls, reader.leaseID)
	}
}

func TestGetModelLeaseAdminDetailFailsClosedWithoutNormalizedReader(t *testing.T) {
	repository := &normalizedWithoutLeaseDetailReader{}
	svc := NewControlPlaneWithRepository(repository)
	if _, err := svc.GetModelLeaseAdminDetail(context.Background(), "lease-1"); !errors.Is(err, store.ErrNormalizedModelLeaseDetailReaderRequired) {
		t.Fatalf("error = %v, want normalized detail reader requirement", err)
	}
	if repository.runCalls != 0 {
		t.Fatalf("normalized detail unexpectedly used StateOperation: %d", repository.runCalls)
	}
}

type normalizedLeaseDetailReader struct {
	detail   controlplane.ModelLeaseAdminDetail
	runCalls int
	leaseID  string
}

func (r *normalizedLeaseDetailReader) Now() time.Time { return time.Unix(0, 0) }

func (r *normalizedLeaseDetailReader) Run(context.Context, store.StateOperation) error {
	r.runCalls++
	return errors.New("state fallback must not be used")
}

func (r *normalizedLeaseDetailReader) UsesNormalizedReadSource() bool { return true }

func (r *normalizedLeaseDetailReader) GetModelLeaseAdminDetail(_ context.Context, leaseID string) (controlplane.ModelLeaseAdminDetail, error) {
	r.leaseID = leaseID
	return r.detail, nil
}

type normalizedWithoutLeaseDetailReader struct {
	runCalls int
}

func (r *normalizedWithoutLeaseDetailReader) Now() time.Time { return time.Unix(0, 0) }

func (r *normalizedWithoutLeaseDetailReader) Run(context.Context, store.StateOperation) error {
	r.runCalls++
	return errors.New("state fallback must not be used")
}

func (r *normalizedWithoutLeaseDetailReader) UsesNormalizedReadSource() bool { return true }
