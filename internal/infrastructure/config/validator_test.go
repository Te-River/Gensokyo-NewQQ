package config

import (
	"errors"
	"strings"
	"testing"
)

// decodeFixture 解析并迁移 fixture 为 DTO（不做校验）。
func decodeFixture(t *testing.T, name string) ConfigDTO {
	t.Helper()
	root, err := ParseNode(readFixture(t, name))
	if err != nil {
		t.Fatalf("ParseNode(%s): %v", name, err)
	}
	if err := Migrate(root, CurrentSchemaVersion); err != nil {
		t.Fatalf("Migrate(%s): %v", name, err)
	}
	var dto ConfigDTO
	if err := root.Decode(&dto); err != nil {
		t.Fatalf("Decode(%s): %v", name, err)
	}
	return dto
}

func assertValidationError(t *testing.T, err error, wantPath string) {
	t.Helper()
	if err == nil {
		t.Fatalf("expected validation error with path %q", wantPath)
	}
	var ce *Error
	if !errors.As(err, &ce) || ce.Kind != KindValidation {
		t.Fatalf("err = %v, want KindValidation", err)
	}
	if !strings.Contains(err.Error(), wantPath) {
		t.Fatalf("error should contain path %q, got: %v", wantPath, err)
	}
}

func TestValidateSchemaRejectsInvalidPort(t *testing.T) {
	err := ValidateSchema(decodeFixture(t, "invalid-port.yml"))
	assertValidationError(t, err, "config.idmap.grpc_port")
}

func TestValidateSchemaRejectsInvalidURL(t *testing.T) {
	err := ValidateSchema(decodeFixture(t, "invalid-url.yml"))
	assertValidationError(t, err, "config.transport.post_url[0]")
}

func TestValidateSchemaRejectsEmptyAppID(t *testing.T) {
	err := ValidateSchema(ConfigDTO{Version: CurrentSchemaVersion})
	assertValidationError(t, err, "config.qq.app_id")
}

func TestValidateSchemaRejectsInvalidOssType(t *testing.T) {
	dto := ConfigDTO{Version: CurrentSchemaVersion}
	dto.Settings.AppID = 12345
	dto.Settings.OssType = 99
	err := ValidateSchema(dto)
	assertValidationError(t, err, "config.media.oss_type")
}

func TestValidateSemanticRejectsMissingSecret(t *testing.T) {
	err := ValidateSemantic(decodeFixture(t, "missing-secret.yml"))
	assertValidationError(t, err, "config.media.image_provider")
}

func TestValidateSemanticRejectsTLSWithoutCert(t *testing.T) {
	dto := ConfigDTO{Version: CurrentSchemaVersion}
	dto.Settings.AppID = 12345
	dto.Settings.UseSelfCrt = true
	err := ValidateSemantic(dto)
	assertValidationError(t, err, "config.transport.tls")
}

func TestValidateSemanticRejectsLotusWithoutEndpoint(t *testing.T) {
	dto := ConfigDTO{Version: CurrentSchemaVersion}
	dto.Settings.AppID = 12345
	dto.Settings.Lotus = true
	err := ValidateSemantic(dto)
	assertValidationError(t, err, "config.lotus.endpoint")
}

func TestValidateValidConfig(t *testing.T) {
	if err := Validate(decodeFixture(t, "v1-basic.yml")); err != nil {
		t.Fatalf("valid config rejected: %v", err)
	}
}
