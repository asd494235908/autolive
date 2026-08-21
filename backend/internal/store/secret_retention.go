package store

import (
	"context"
	"errors"
	"strings"

	"github.com/lib/pq"
)

const cleanupStagedSecretsSQL = `
	WITH candidates AS (
		SELECT secret_ref
		FROM model_account_secrets
		WHERE secret_ref LIKE 'model-account/%/rotation_%'
		  AND updated_at < $1
		  AND NOT (secret_ref = ANY($2::text[]))
		ORDER BY updated_at, secret_ref
		LIMIT $3
		FOR UPDATE SKIP LOCKED
	)
	DELETE FROM model_account_secrets AS target
	USING candidates
	WHERE target.secret_ref = candidates.secret_ref
`

func (s *EncryptedSQLSecretStore) CleanupStagedSecrets(ctx context.Context, request RetentionCleanupRequest, protectedReferences []string) (int64, error) {
	if err := validateRetentionCleanupRequest(request); err != nil {
		return 0, err
	}
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	result, err := s.db.ExecContext(operationCtx, cleanupStagedSecretsSQL, request.Cutoff.UTC(), pq.Array(protectedReferences), request.BatchSize)
	if err != nil {
		return 0, postgresOperationError(operationCtx, err)
	}
	deleted, err := result.RowsAffected()
	if err != nil {
		return 0, postgresOperationError(operationCtx, err)
	}
	return deleted, nil
}

const cleanupSecretReferencesSQL = `
WITH candidates AS (
	SELECT secret_ref
	FROM model_account_secrets
	WHERE secret_ref = ANY($1::text[])
	  AND updated_at < $2
	  AND NOT (secret_ref = ANY($3::text[]))
	ORDER BY updated_at, secret_ref
	LIMIT $4
	FOR UPDATE SKIP LOCKED
)
DELETE FROM model_account_secrets AS target
USING candidates
WHERE target.secret_ref = candidates.secret_ref
RETURNING target.secret_ref
`

func (s *EncryptedSQLSecretStore) CleanupSecretReferences(ctx context.Context, request RetentionCleanupRequest, references, protectedReferences []string) ([]string, error) {
	if err := validateRetentionCleanupRequest(request); err != nil {
		return nil, err
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	references = normalizeSecretReferences(references, request.BatchSize)
	if len(references) == 0 {
		return nil, nil
	}
	protected := make([]string, 0, len(protectedReferences))
	for _, reference := range protectedReferences {
		reference = strings.TrimSpace(reference)
		if reference != "" {
			protected = append(protected, reference)
		}
	}
	operationCtx, cancel := s.operationContext(ctx)
	defer cancel()
	rows, err := s.db.QueryContext(operationCtx, cleanupSecretReferencesSQL, pq.Array(references), request.Cutoff.UTC(), pq.Array(protected), request.BatchSize)
	if err != nil {
		return nil, postgresOperationError(operationCtx, err)
	}
	defer rows.Close()
	deleted := make([]string, 0, len(references))
	for rows.Next() {
		var reference string
		if err := rows.Scan(&reference); err != nil {
			return nil, postgresOperationError(operationCtx, err)
		}
		deleted = append(deleted, reference)
	}
	if err := rows.Err(); err != nil {
		return nil, postgresOperationError(operationCtx, err)
	}
	if err := rows.Close(); err != nil && !errors.Is(err, context.Canceled) {
		return nil, postgresOperationError(operationCtx, err)
	}
	return deleted, nil
}
