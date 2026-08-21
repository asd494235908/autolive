//go:build postgres_integration

package store

import (
	"strings"
	"testing"
)

func TestPostgresManagementFilterIndexesAreVisibleToPlanner(t *testing.T) {
	database, ctx := openPostgresIntegrationDatabase(t)
	tx, err := database.BeginTx(ctx, nil)
	if err != nil {
		t.Fatalf("begin planner transaction: %v", err)
	}
	defer func() { _ = tx.Rollback() }()
	if _, err := tx.ExecContext(ctx, "SET LOCAL enable_seqscan = off"); err != nil {
		t.Fatalf("disable sequential scans: %v", err)
	}
	if _, err := tx.ExecContext(ctx, "SET LOCAL enable_bitmapscan = off"); err != nil {
		t.Fatalf("disable bitmap scans: %v", err)
	}

	cases := []struct {
		name  string
		query string
		want  string
		args  []any
	}{
		{
			name: "lease user status",
			query: `EXPLAIN (COSTS OFF)
				SELECT id FROM model_leases
				WHERE user_id = $1 AND status = $2
				ORDER BY expires_at, id LIMIT 20`,
			want: "idx_model_leases_user_status_expiry",
			args: []any{"missing-user", "active"},
		},
		{
			name: "audit actor",
			query: `EXPLAIN (COSTS OFF)
				SELECT id FROM audit_logs
				WHERE actor_user_id = $1
				ORDER BY created_at DESC, id DESC LIMIT 20`,
			want: "idx_audit_logs_actor_created_at",
			args: []any{"missing-actor"},
		},
	}
	for _, testCase := range cases {
		t.Run(testCase.name, func(t *testing.T) {
			rows, err := tx.QueryContext(ctx, testCase.query, testCase.args...)
			if err != nil {
				t.Fatalf("EXPLAIN query: %v", err)
			}
			defer rows.Close()
			var plan strings.Builder
			for rows.Next() {
				var line string
				if err := rows.Scan(&line); err != nil {
					t.Fatalf("scan EXPLAIN row: %v", err)
				}
				plan.WriteString(line)
				plan.WriteByte('\n')
			}
			if err := rows.Err(); err != nil {
				t.Fatalf("EXPLAIN rows: %v", err)
			}
			if !strings.Contains(plan.String(), testCase.want) {
				t.Fatalf("EXPLAIN plan = %s, want index %q", plan.String(), testCase.want)
			}
		})
	}
}
