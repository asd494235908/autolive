package store

import (
	"context"
	"database/sql"
	"errors"
	"strings"
	"time"

	"autoLive/backend/internal/controlplane"
)

var _ ProductRepository = (*PostgresRepository)(nil)

var ErrNormalizedProductRepositoryRequired = errors.New("normalized product repository is required")
var ErrProductNotFound = errors.New("product not found")

const (
	listProductsQuery = `SELECT code, status, created_at
		FROM products
		ORDER BY code`
	getProductQuery = `SELECT code, status, created_at
		FROM products
		WHERE code = $1`
	getUserProductMembershipQuery = `SELECT user_id, product, status, entitlement_revision, created_at, updated_at
		FROM user_products
		WHERE user_id = $1 AND product = $2`
	insertUserProductMembershipQuery = `INSERT INTO user_products (user_id, product, status, entitlement_revision, created_at, updated_at)
		VALUES ($1, $2, $3, $4, $5, $5)
		ON CONFLICT (user_id, product) DO NOTHING`
)

type sqlScanner interface {
	Scan(dest ...any) error
}

func (s *PostgresRepository) ListProducts(ctx context.Context) ([]controlplane.ProductSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return nil, ErrNormalizedProductRepositoryRequired
	}
	if ctx == nil {
		return nil, controlplane.ErrInvalidRequest
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	rows, err := s.db.QueryContext(operationCtx, listProductsQuery)
	if err != nil {
		return nil, postgresOperationError(operationCtx, err)
	}
	defer rows.Close()

	products := make([]controlplane.ProductSummary, 0)
	for rows.Next() {
		product, err := scanProductSummary(rows)
		if err != nil {
			return nil, postgresOperationError(operationCtx, err)
		}
		products = append(products, product)
	}
	if err := rows.Err(); err != nil {
		return nil, postgresOperationError(operationCtx, err)
	}
	return products, nil
}

func (s *PostgresRepository) GetProduct(ctx context.Context, product controlplane.ProductCode) (controlplane.ProductSummary, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.ProductSummary{}, ErrNormalizedProductRepositoryRequired
	}
	if ctx == nil {
		return controlplane.ProductSummary{}, controlplane.ErrInvalidRequest
	}
	if !product.Valid() {
		return controlplane.ProductSummary{}, ErrProductNotFound
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	summary, err := scanProductSummary(s.db.QueryRowContext(operationCtx, getProductQuery, string(product)))
	if errors.Is(err, sql.ErrNoRows) {
		return controlplane.ProductSummary{}, ErrProductNotFound
	}
	if err != nil {
		return controlplane.ProductSummary{}, postgresOperationError(operationCtx, err)
	}
	return summary, nil
}

func (s *PostgresRepository) GetUserProductMembership(ctx context.Context, userID string, product controlplane.ProductCode) (controlplane.UserProductMembership, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.UserProductMembership{}, ErrNormalizedProductRepositoryRequired
	}
	if ctx == nil {
		return controlplane.UserProductMembership{}, controlplane.ErrInvalidRequest
	}
	userID = strings.TrimSpace(userID)
	if userID == "" || !product.Valid() {
		return controlplane.UserProductMembership{}, controlplane.ErrForbidden
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	membership, err := scanUserProductMembership(s.db.QueryRowContext(operationCtx, getUserProductMembershipQuery, userID, string(product)))
	if errors.Is(err, sql.ErrNoRows) {
		return controlplane.UserProductMembership{}, controlplane.ErrForbidden
	}
	if err != nil {
		return controlplane.UserProductMembership{}, postgresOperationError(operationCtx, err)
	}
	return membership, nil
}

func (s *PostgresRepository) EnsureUserProductMembership(ctx context.Context, userID string, product controlplane.ProductCode) (controlplane.UserProductMembership, error) {
	if s.modelReadSource != ModelReadSourceNormalized {
		return controlplane.UserProductMembership{}, ErrNormalizedProductRepositoryRequired
	}
	if ctx == nil {
		return controlplane.UserProductMembership{}, controlplane.ErrInvalidRequest
	}
	userID = strings.TrimSpace(userID)
	if userID == "" {
		return controlplane.UserProductMembership{}, controlplane.ErrUserNotFound
	}
	if !product.Valid() {
		return controlplane.UserProductMembership{}, ErrProductNotFound
	}

	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	tx, err := s.db.BeginTx(operationCtx, nil)
	if err != nil {
		return controlplane.UserProductMembership{}, postgresOperationError(operationCtx, err)
	}
	defer func() { _ = tx.Rollback() }()

	productSummary, err := scanProductSummary(tx.QueryRowContext(operationCtx, getProductQuery, string(product)))
	if errors.Is(err, sql.ErrNoRows) {
		return controlplane.UserProductMembership{}, ErrProductNotFound
	}
	if err != nil {
		return controlplane.UserProductMembership{}, postgresOperationError(operationCtx, err)
	}
	if productSummary.Status != "active" {
		return controlplane.UserProductMembership{}, ErrProductNotFound
	}

	var userExists bool
	if err := tx.QueryRowContext(operationCtx, `SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)`, userID).Scan(&userExists); err != nil {
		return controlplane.UserProductMembership{}, postgresOperationError(operationCtx, err)
	}
	if !userExists {
		return controlplane.UserProductMembership{}, controlplane.ErrUserNotFound
	}

	now := s.Now()
	if _, err := tx.ExecContext(operationCtx, insertUserProductMembershipQuery, userID, string(product), "active", int64(0), now); err != nil {
		return controlplane.UserProductMembership{}, postgresOperationError(operationCtx, err)
	}

	membership, err := scanUserProductMembership(tx.QueryRowContext(operationCtx, getUserProductMembershipQuery, userID, string(product)))
	if errors.Is(err, sql.ErrNoRows) {
		return controlplane.UserProductMembership{}, controlplane.ErrForbidden
	}
	if err != nil {
		return controlplane.UserProductMembership{}, postgresOperationError(operationCtx, err)
	}
	if err := tx.Commit(); err != nil {
		return controlplane.UserProductMembership{}, postgresCommitError(operationCtx, "commit normalized product membership", err)
	}
	return membership, nil
}

func scanProductSummary(scanner sqlScanner) (controlplane.ProductSummary, error) {
	var summary controlplane.ProductSummary
	var createdAt time.Time
	if err := scanner.Scan(&summary.Code, &summary.Status, &createdAt); err != nil {
		return controlplane.ProductSummary{}, err
	}
	summary.CreatedAt = createdAt.UTC().Format(time.RFC3339)
	return summary, nil
}

func scanUserProductMembership(scanner sqlScanner) (controlplane.UserProductMembership, error) {
	var membership controlplane.UserProductMembership
	var createdAt time.Time
	var updatedAt time.Time
	if err := scanner.Scan(
		&membership.UserID,
		&membership.Product,
		&membership.Status,
		&membership.EntitlementRevision,
		&createdAt,
		&updatedAt,
	); err != nil {
		return controlplane.UserProductMembership{}, err
	}
	membership.CreatedAt = createdAt.UTC().Format(time.RFC3339)
	membership.UpdatedAt = updatedAt.UTC().Format(time.RFC3339)
	return membership, nil
}
