package store

import (
	"context"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresRepositoryListActivationCodesPageUsesBoundedNormalizedQuery(t *testing.T) {
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	defer database.Close()
	now := time.Date(2026, 8, 21, 16, 0, 0, 0, time.UTC)
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time { return now }, nil, ModelReadSourceNormalized)
	if err != nil {
		t.Fatalf("constructor error = %v", err)
	}

	mock.ExpectBegin()
	expectNormalizedPageCoverage(mock)
	mock.ExpectQuery(regexp.QuoteMeta("SELECT COUNT(*) FROM activation_codes")).WillReturnRows(sqlmock.NewRows([]string{"count"}).AddRow(3))
	mock.ExpectQuery(regexp.QuoteMeta("SELECT id, code_prefix")).WithArgs(controlplane.ActivationCodeStatusActive, now, controlplane.ActivationCodeStatusExpired, 2, 1).WillReturnRows(
		sqlmock.NewRows([]string{"id", "code_prefix", "status", "expires_at", "used_at", "used_by_user_id", "used_by_device_id", "max_devices", "bound_devices"}).
			AddRow("ac_2", "code_abcdef", controlplane.ActivationCodeStatusExpired, now.Add(-time.Hour), nil, nil, nil, 1, 0).
			AddRow("ac_3", "code_ghijkl", controlplane.ActivationCodeStatusUsed, now.Add(time.Hour), now.Add(-time.Minute), "usr_1", "dev_1", 3, 3),
	)
	mock.ExpectCommit()

	page, err := repository.ListActivationCodesPage(context.Background(), 1, 2)
	if err != nil {
		t.Fatalf("ListActivationCodesPage() error = %v", err)
	}
	if page.Total != 3 || len(page.Items) != 2 || page.Items[0].ID != "ac_2" || page.Items[0].PlainCode != nil || page.Items[0].MaxDevices != 1 || page.Items[0].BoundDevices != 0 || page.Items[1].UsedByUserID != "usr_1" || page.Items[1].MaxDevices != 3 || page.Items[1].BoundDevices != 3 {
		t.Fatalf("activation page = %+v", page)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}
