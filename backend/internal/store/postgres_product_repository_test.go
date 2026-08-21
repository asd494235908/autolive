package store

import (
	"context"
	"database/sql"
	"errors"
	"regexp"
	"testing"
	"time"

	"autoLive/backend/internal/controlplane"
	"github.com/DATA-DOG/go-sqlmock"
)

func TestPostgresProductRepositoryListsRegisteredProducts(t *testing.T) {
	database, mock, repository := newProductRepositoryTest(t)
	defer database.Close()

	now := time.Date(2026, 8, 22, 1, 0, 0, 0, time.UTC)
	mock.ExpectQuery(regexp.QuoteMeta(`SELECT code, status, created_at
		FROM products
		ORDER BY code`)).WillReturnRows(sqlmock.NewRows([]string{"code", "status", "created_at"}).
		AddRow(string(controlplane.ProductAutoLive), "active", now).
		AddRow(string(controlplane.ProductDouyinDesktop), "active", now))

	products, err := repository.ListProducts(context.Background())
	if err != nil {
		t.Fatalf("ListProducts() error = %v", err)
	}
	if len(products) != 2 || products[0].Code != controlplane.ProductAutoLive || products[1].Code != controlplane.ProductDouyinDesktop {
		t.Fatalf("products = %+v", products)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresProductRepositoryMissingProductReturnsNotFound(t *testing.T) {
	database, mock, repository := newProductRepositoryTest(t)
	defer database.Close()

	mock.ExpectQuery(regexp.QuoteMeta(`SELECT code, status, created_at
		FROM products
		WHERE code = $1`)).WithArgs(string(controlplane.ProductDouyinDesktop)).WillReturnError(sql.ErrNoRows)

	if _, err := repository.GetProduct(context.Background(), controlplane.ProductDouyinDesktop); !errors.Is(err, ErrProductNotFound) {
		t.Fatalf("GetProduct() error = %v, want ErrProductNotFound", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresProductRepositoryCrossProductMembershipReturnsForbidden(t *testing.T) {
	database, mock, repository := newProductRepositoryTest(t)
	defer database.Close()

	mock.ExpectQuery(regexp.QuoteMeta(`SELECT user_id, product, status, entitlement_revision, created_at, updated_at
		FROM user_products
		WHERE user_id = $1 AND product = $2`)).WithArgs("usr_1", string(controlplane.ProductDouyinDesktop)).WillReturnError(sql.ErrNoRows)

	if _, err := repository.GetUserProductMembership(context.Background(), "usr_1", controlplane.ProductDouyinDesktop); !errors.Is(err, controlplane.ErrForbidden) {
		t.Fatalf("GetUserProductMembership() error = %v, want forbidden", err)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresProductRepositoryEnsureMembershipIsIdempotent(t *testing.T) {
	database, mock, repository := newProductRepositoryTest(t)
	defer database.Close()

	now := time.Date(2026, 8, 22, 1, 0, 0, 0, time.UTC)
	expectEnsureMembership(mock, now, true)
	created, err := repository.EnsureUserProductMembership(context.Background(), "usr_1", controlplane.ProductDouyinDesktop)
	if err != nil {
		t.Fatalf("first EnsureUserProductMembership() error = %v", err)
	}
	if created.UserID != "usr_1" || created.Product != controlplane.ProductDouyinDesktop || created.Status != "active" {
		t.Fatalf("created membership = %+v", created)
	}

	expectEnsureMembership(mock, now, false)
	replayed, err := repository.EnsureUserProductMembership(context.Background(), "usr_1", controlplane.ProductDouyinDesktop)
	if err != nil {
		t.Fatalf("second EnsureUserProductMembership() error = %v", err)
	}
	if replayed != created {
		t.Fatalf("replayed membership = %+v, want %+v", replayed, created)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func TestPostgresProductRepositoryPreservesDisabledMembership(t *testing.T) {
	database, mock, repository := newProductRepositoryTest(t)
	defer database.Close()

	now := time.Date(2026, 8, 22, 1, 0, 0, 0, time.UTC)
	mock.ExpectQuery(regexp.QuoteMeta(`SELECT user_id, product, status, entitlement_revision, created_at, updated_at
		FROM user_products
		WHERE user_id = $1 AND product = $2`)).WithArgs("usr_1", string(controlplane.ProductAutoLive)).WillReturnRows(
		sqlmock.NewRows([]string{"user_id", "product", "status", "entitlement_revision", "created_at", "updated_at"}).
			AddRow("usr_1", string(controlplane.ProductAutoLive), "disabled", int64(4), now, now),
	)

	membership, err := repository.GetUserProductMembership(context.Background(), "usr_1", controlplane.ProductAutoLive)
	if err != nil {
		t.Fatalf("GetUserProductMembership() error = %v", err)
	}
	if membership.Status != "disabled" || membership.EntitlementRevision != 4 {
		t.Fatalf("membership = %+v", membership)
	}
	if err := mock.ExpectationsWereMet(); err != nil {
		t.Fatalf("sql expectations: %v", err)
	}
}

func newProductRepositoryTest(t *testing.T) (*sql.DB, sqlmock.Sqlmock, *PostgresRepository) {
	t.Helper()
	database, mock, err := sqlmock.New()
	if err != nil {
		t.Fatalf("sqlmock.New() error = %v", err)
	}
	repository, err := NewPostgresRepositoryWithSecretStoreAndModelReadSource(database, func() time.Time {
		return time.Date(2026, 8, 22, 1, 0, 0, 0, time.UTC)
	}, nil, ModelReadSourceNormalized)
	if err != nil {
		database.Close()
		t.Fatalf("repository constructor error = %v", err)
	}
	return database, mock, repository
}

func expectEnsureMembership(mock sqlmock.Sqlmock, now time.Time, inserted bool) {
	mock.ExpectBegin()
	mock.ExpectQuery(regexp.QuoteMeta(`SELECT code, status, created_at
		FROM products
		WHERE code = $1`)).WithArgs(string(controlplane.ProductDouyinDesktop)).WillReturnRows(
		sqlmock.NewRows([]string{"code", "status", "created_at"}).AddRow(string(controlplane.ProductDouyinDesktop), "active", now),
	)
	mock.ExpectQuery(regexp.QuoteMeta(`SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)`)).WithArgs("usr_1").WillReturnRows(
		sqlmock.NewRows([]string{"exists"}).AddRow(true),
	)
	insert := mock.ExpectExec(regexp.QuoteMeta(`INSERT INTO user_products (user_id, product, status, entitlement_revision, created_at, updated_at)
		VALUES ($1, $2, $3, $4, $5, $5)
		ON CONFLICT (user_id, product) DO NOTHING`)).WithArgs("usr_1", string(controlplane.ProductDouyinDesktop), "active", int64(0), now)
	if inserted {
		insert.WillReturnResult(sqlmock.NewResult(1, 1))
	} else {
		insert.WillReturnResult(sqlmock.NewResult(1, 0))
	}
	mock.ExpectQuery(regexp.QuoteMeta(`SELECT user_id, product, status, entitlement_revision, created_at, updated_at
		FROM user_products
		WHERE user_id = $1 AND product = $2`)).WithArgs("usr_1", string(controlplane.ProductDouyinDesktop)).WillReturnRows(
		sqlmock.NewRows([]string{"user_id", "product", "status", "entitlement_revision", "created_at", "updated_at"}).
			AddRow("usr_1", string(controlplane.ProductDouyinDesktop), "active", int64(0), now, now),
	)
	mock.ExpectCommit()
}
