package service_test

import "testing"

func derefString(t *testing.T, value *string) string {
	t.Helper()
	if value == nil {
		t.Fatal("expected non-nil string pointer")
	}
	return *value
}
