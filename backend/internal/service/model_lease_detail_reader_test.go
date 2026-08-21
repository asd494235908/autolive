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

func TestGetModelLeaseAdminDetailForProductUsesStrictReader(t *testing.T) {
	reader := &normalizedProductLeaseDetailReader{detail: controlplane.ModelLeaseAdminDetail{ID: "lease-1", Product: controlplane.ProductDouyinDesktop}}
	svc := NewControlPlaneWithRepository(reader)
	detail, err := svc.GetModelLeaseAdminDetailForProduct(context.Background(), "lease-1", controlplane.ProductDouyinDesktop)
	if err != nil {
		t.Fatalf("GetModelLeaseAdminDetailForProduct() error = %v", err)
	}
	if detail.Product != controlplane.ProductDouyinDesktop || reader.product != controlplane.ProductDouyinDesktop {
		t.Fatalf("detail=%+v product=%q", detail, reader.product)
	}
}

func TestGetModelLeaseAdminDetailForProductFailsClosedWithoutStrictReader(t *testing.T) {
	reader := &normalizedLeaseDetailReader{detail: controlplane.ModelLeaseAdminDetail{ID: "lease-1"}}
	svc := NewControlPlaneWithRepository(reader)
	if _, err := svc.GetModelLeaseAdminDetailForProduct(context.Background(), "lease-1", controlplane.ProductDouyinDesktop); !errors.Is(err, store.ErrNormalizedModelLeaseDetailReaderRequired) {
		t.Fatalf("error = %v, want normalized strict detail reader requirement", err)
	}
}

type normalizedLeaseDetailReader struct {
	detail   controlplane.ModelLeaseAdminDetail
	runCalls int
	leaseID  string
}

type normalizedProductLeaseDetailReader struct {
	detail  controlplane.ModelLeaseAdminDetail
	product controlplane.ProductCode
}

func (r *normalizedProductLeaseDetailReader) Now() time.Time { return time.Unix(0, 0) }

func (r *normalizedProductLeaseDetailReader) Run(context.Context, store.StateOperation) error {
	return errors.New("state fallback must not be used")
}

func (r *normalizedProductLeaseDetailReader) UsesNormalizedReadSource() bool { return true }

func (r *normalizedProductLeaseDetailReader) GetModelLeaseAdminDetailForProduct(_ context.Context, _ string, product controlplane.ProductCode) (controlplane.ModelLeaseAdminDetail, error) {
	r.product = product
	return r.detail, nil
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
