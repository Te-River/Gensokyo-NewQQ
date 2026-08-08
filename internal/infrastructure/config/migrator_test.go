package config

import "testing"

func TestMigrateLegacyAddsVersion(t *testing.T) {
	root, err := ParseNode(readFixture(t, "legacy-basic.yml"))
	if err != nil {
		t.Fatalf("ParseNode: %v", err)
	}
	if v := detectVersion(root); v != 0 {
		t.Fatalf("detectVersion(legacy) = %d, want 0", v)
	}

	if err := Migrate(root, CurrentSchemaVersion); err != nil {
		t.Fatalf("Migrate: %v", err)
	}
	if v := detectVersion(root); v != CurrentSchemaVersion {
		t.Fatalf("version after migrate = %d, want %d", v, CurrentSchemaVersion)
	}

	var dto ConfigDTO
	if err := root.Decode(&dto); err != nil {
		t.Fatalf("Decode after migrate: %v", err)
	}
	if dto.Version != CurrentSchemaVersion {
		t.Fatalf("dto.Version = %d, want %d", dto.Version, CurrentSchemaVersion)
	}
}

func TestMigrateV1Noop(t *testing.T) {
	root, err := ParseNode(readFixture(t, "v1-basic.yml"))
	if err != nil {
		t.Fatalf("ParseNode: %v", err)
	}
	if err := Migrate(root, CurrentSchemaVersion); err != nil {
		t.Fatalf("Migrate on v1 should be no-op: %v", err)
	}
	if v := detectVersion(root); v != CurrentSchemaVersion {
		t.Fatalf("version = %d, want %d", v, CurrentSchemaVersion)
	}
}

func TestMigrateLegacyFullKeepsFields(t *testing.T) {
	root, err := ParseNode(readFixture(t, "legacy-full.yml"))
	if err != nil {
		t.Fatalf("ParseNode: %v", err)
	}
	if err := Migrate(root, CurrentSchemaVersion); err != nil {
		t.Fatalf("Migrate: %v", err)
	}
	var dto ConfigDTO
	if err := root.Decode(&dto); err != nil {
		t.Fatalf("Decode: %v", err)
	}
	if dto.Settings.HttpAddress != "0.0.0.0:5700" {
		t.Fatalf("http_address lost during migration: %q", dto.Settings.HttpAddress)
	}
	if len(dto.Settings.WsAddress) != 2 {
		t.Fatalf("ws_address lost during migration: %v", dto.Settings.WsAddress)
	}
}
