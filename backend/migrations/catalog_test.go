package migrations

import (
	"io/fs"
	"testing"
)

func TestEmbeddedMigrationsUseVersionedUpSQLFiles(t *testing.T) {
	entries, err := fs.ReadDir(FS, ".")
	if err != nil {
		t.Fatalf("fs.ReadDir() error = %v", err)
	}
	if len(entries) == 0 {
		t.Fatal("embedded migration catalog is empty")
	}
	for _, entry := range entries {
		if entry.IsDir() || len(entry.Name()) < 8 || entry.Name()[:4] < "0001" {
			t.Fatalf("invalid migration entry: %s", entry.Name())
		}
		if len(entry.Name()) < 7 || entry.Name()[len(entry.Name())-7:] != ".up.sql" {
			t.Fatalf("migration %s must end with .up.sql", entry.Name())
		}
	}
}

func TestLatestVersionMatchesEmbeddedCatalog(t *testing.T) {
	entries, err := fs.ReadDir(FS, ".")
	if err != nil {
		t.Fatalf("fs.ReadDir() error = %v", err)
	}
	if len(entries) != LatestVersion {
		t.Fatalf("embedded migration count = %d, want latest version %d", len(entries), LatestVersion)
	}
}
