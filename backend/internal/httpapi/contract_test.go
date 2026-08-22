package httpapi

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"runtime"
	"slices"
	"sort"
	"strconv"
	"strings"
	"testing"

	"autoLive/backend/internal/controlplane"
	"autoLive/backend/internal/store"
	"gopkg.in/yaml.v3"
)

var openAPIPathParameter = regexp.MustCompile(`\{[^}]+\}`)
var errorCodeRow = regexp.MustCompile(`^\|\s*` + "`" + `([A-Z][A-Z0-9_]*)` + "`" + `\s*\|`)
var errorCodeStatusRow = regexp.MustCompile(`^\|\s*` + "`" + `([A-Z][A-Z0-9_]*)` + "`" + `\s*\|\s*` + "`?" + `([0-9]{3})` + "`?")

type openAPIDocument struct {
	Paths      map[string]map[string]yaml.Node `yaml:"paths"`
	Components map[string]map[string]yaml.Node `yaml:"components"`
}

func loadOpenAPIContract(t *testing.T) openAPIDocument {
	t.Helper()
	_, sourceFile, _, ok := runtime.Caller(1)
	if !ok {
		t.Fatal("runtime.Caller() failed")
	}
	contractPath := filepath.Clean(filepath.Join(filepath.Dir(sourceFile), "..", "..", "..", "接口契约", "openapi.yaml"))
	contract, err := os.ReadFile(contractPath)
	if err != nil {
		t.Fatalf("read OpenAPI contract %q: %v", contractPath, err)
	}
	var document openAPIDocument
	if err := yaml.Unmarshal(contract, &document); err != nil {
		t.Fatalf("decode OpenAPI contract: %v", err)
	}
	return document
}

func TestOpenAPIRoutesAreRegisteredByHTTPRouter(t *testing.T) {
	document := loadOpenAPIContract(t)
	if len(document.Paths) == 0 {
		t.Fatal("OpenAPI contract contains no paths")
	}

	handler := NewRouterWithAuth("contract-test", slog.New(slog.NewTextHandler(io.Discard, nil)), AuthConfig{
		Username: "admin",
		Password: "contract-test-password",
	})
	methods := map[string]struct{}{
		"get": {}, "post": {}, "put": {}, "patch": {}, "delete": {}, "head": {}, "options": {},
	}
	for pathTemplate, operations := range document.Paths {
		path := openAPIPathParameter.ReplaceAllString(pathTemplate, "resource_00000001")
		for method := range operations {
			if _, ok := methods[method]; !ok {
				continue
			}
			t.Run(strings.ToUpper(method)+" "+pathTemplate, func(t *testing.T) {
				request := httptest.NewRequest(strings.ToUpper(method), path, nil)
				request.RemoteAddr = "192.0.2.20:1234"
				recorder := httptest.NewRecorder()
				handler.ServeHTTP(recorder, request)
				if recorder.Code == http.StatusNotFound || recorder.Code == http.StatusMethodNotAllowed {
					t.Fatalf("OpenAPI route %s %s returned %d; route is not registered", method, pathTemplate, recorder.Code)
				}
				if requestID := recorder.Header().Get("X-Request-Id"); requestID == "" {
					t.Fatal("response is missing X-Request-Id")
				}
				if contentType := recorder.Header().Get("Content-Type"); !strings.Contains(contentType, "application/json") {
					t.Fatalf("response content type %q is not application/json", contentType)
				}
			})
		}
	}
}

func TestOpenAPILocalReferencesResolve(t *testing.T) {
	document := loadOpenAPIContract(t)
	if len(document.Components) == 0 {
		t.Fatal("OpenAPI contract contains no components")
	}

	var unresolved []string
	var inspect func(yaml.Node, string)
	inspect = func(node yaml.Node, location string) {
		if node.Kind == yaml.MappingNode {
			for index := 0; index+1 < len(node.Content); index += 2 {
				key := node.Content[index]
				value := node.Content[index+1]
				if key.Value == "$ref" && strings.HasPrefix(value.Value, "#/components/") {
					parts := strings.SplitN(strings.TrimPrefix(value.Value, "#/components/"), "/", 2)
					if len(parts) != 2 {
						unresolved = append(unresolved, location+": "+value.Value)
					} else if _, ok := document.Components[parts[0]][parts[1]]; !ok {
						unresolved = append(unresolved, location+": "+value.Value)
					}
				}
				inspect(*value, location+"/"+key.Value)
			}
			return
		}
		for _, child := range node.Content {
			inspect(*child, location)
		}
	}

	for path, operations := range document.Paths {
		for method, operation := range operations {
			inspect(operation, path+" "+method)
		}
	}
	for section, entries := range document.Components {
		for name, entry := range entries {
			inspect(entry, "components/"+section+"/"+name)
		}
	}
	sort.Strings(unresolved)
	if len(unresolved) > 0 {
		t.Fatalf("OpenAPI local reference drift: %s", strings.Join(unresolved, "; "))
	}
}

func TestOpenAPIOperationsDeclareCommonRuntimeErrorStatuses(t *testing.T) {
	document := loadOpenAPIContract(t)
	const (
		methodNotAllowed = "MethodNotAllowed"
		requestTimeout   = "RequestTimeout"
		internalError    = "InternalServerError"
	)
	want := map[string]string{
		"405": methodNotAllowed,
		"408": requestTimeout,
		"500": internalError,
	}
	operationCount := 0
	for pathTemplate, operations := range document.Paths {
		for method, operation := range operations {
			if !slices.Contains([]string{"get", "post", "put", "patch", "delete", "head", "options"}, method) {
				continue
			}
			operationCount++
			var contract struct {
				Responses map[string]yaml.Node `yaml:"responses"`
			}
			if err := operation.Decode(&contract); err != nil {
				t.Errorf("decode %s %s operation: %v", method, pathTemplate, err)
				continue
			}
			for status, responseName := range want {
				response, ok := contract.Responses[status]
				if !ok {
					t.Errorf("%s %s is missing common runtime status %s", method, pathTemplate, status)
					continue
				}
				if ref := localReferenceName(response); ref != responseName {
					t.Errorf("%s %s status %s ref = %q, want %q", method, pathTemplate, status, ref, responseName)
				}
			}
		}
	}
	if operationCount == 0 {
		t.Fatal("OpenAPI contract contains no HTTP operations")
	}
}

func TestOpenAPIAdminListGetsDeclareBadRequest(t *testing.T) {
	document := loadOpenAPIContract(t)
	want := []string{
		"/api/v1/admin/users",
		"/api/v1/admin/users/{user_id}/devices",
		"/api/v1/admin/devices",
		"/api/v1/admin/activation-codes",
		"/api/v1/admin/model-pool",
		"/api/v1/admin/model-usage",
		"/api/v1/admin/model-leases",
		"/api/v1/admin/audit-logs",
	}
	for _, path := range want {
		var operation struct {
			Responses map[string]yaml.Node `yaml:"responses"`
		}
		node, ok := document.Paths[path]["get"]
		if !ok {
			t.Fatalf("GET %s is missing", path)
		}
		if err := node.Decode(&operation); err != nil {
			t.Fatalf("decode GET %s: %v", path, err)
		}
		if ref := localReferenceName(operation.Responses["400"]); ref != "BadRequest" {
			t.Errorf("GET %s status 400 ref = %q, want BadRequest", path, ref)
		}
	}
}

func TestOpenAPIAdminPermissionGuardRoutesDeclareServiceUnavailable(t *testing.T) {
	document := loadOpenAPIContract(t)
	required := []struct {
		method string
		path   string
	}{
		{method: "get", path: "/api/v1/admin/users"},
		{method: "get", path: "/api/v1/admin/users/{user_id}/devices"},
		{method: "get", path: "/api/v1/admin/devices/{device_id}"},
		{method: "post", path: "/api/v1/admin/activation-codes/{code_id}/revoke"},
		{method: "get", path: "/api/v1/admin/model-pool"},
		{method: "post", path: "/api/v1/admin/model-pool/{account_id}/disable"},
		{method: "patch", path: "/api/v1/admin/model-pool/{account_id}"},
		{method: "post", path: "/api/v1/admin/model-pool/{account_id}/test"},
		{method: "get", path: "/api/v1/admin/model-usage"},
		{method: "get", path: "/api/v1/admin/model-leases"},
		{method: "get", path: "/api/v1/admin/model-leases/{lease_id}"},
		{method: "post", path: "/api/v1/admin/model-leases/{lease_id}/reclaim"},
		{method: "get", path: "/api/v1/admin/audit-logs"},
		{method: "get", path: "/api/v1/admin/roles"},
		{method: "post", path: "/api/v1/admin/roles"},
		{method: "get", path: "/api/v1/admin/roles/{role_id}"},
		{method: "patch", path: "/api/v1/admin/roles/{role_id}"},
		{method: "delete", path: "/api/v1/admin/roles/{role_id}"},
		{method: "get", path: "/api/v1/admin/users/{user_id}/roles"},
		{method: "put", path: "/api/v1/admin/users/{user_id}/roles"},
	}

	for _, test := range required {
		t.Run(strings.ToUpper(test.method)+" "+test.path, func(t *testing.T) {
			var operation struct {
				Responses map[string]yaml.Node `yaml:"responses"`
			}
			node, ok := document.Paths[test.path][test.method]
			if !ok {
				t.Fatalf("%s %s is missing", strings.ToUpper(test.method), test.path)
			}
			if err := node.Decode(&operation); err != nil {
				t.Fatalf("decode %s %s: %v", test.method, test.path, err)
			}
			if ref := localReferenceName(operation.Responses["503"]); ref != "ServiceUnavailable" {
				t.Fatalf("%s %s status 503 ref = %q, want ServiceUnavailable", test.method, test.path, ref)
			}
		})
	}
}

func TestOpenAPIProductResourceSchemasRequireProduct(t *testing.T) {
	document := loadOpenAPIContract(t)
	for _, name := range []string{"DeviceSummary", "ActivationCode", "ModelPoolAccountSummary", "ModelLease", "ModelLeaseAdminSummary", "ModelLeaseAdminDetail", "ModelUsageRecord", "AuditLog"} {
		node, ok := document.Components["schemas"][name]
		if !ok {
			t.Fatalf("schema %s is missing", name)
		}
		var schema struct {
			Required []string `yaml:"required"`
		}
		if err := node.Decode(&schema); err != nil {
			t.Fatalf("decode schema %s: %v", name, err)
		}
		if !slices.Contains(schema.Required, "product") {
			t.Errorf("schema %s required fields = %v, want product", name, schema.Required)
		}
	}
}

func TestOpenAPIMirroredDTOFieldsMatchGoJSONTags(t *testing.T) {
	document := loadOpenAPIContract(t)
	// These are the DTOs that cross the Go HTTP boundary. Keeping the mapping
	// explicit makes a renamed or removed Go type fail the contract test instead
	// of silently dropping a field from the wire format.
	types := map[string]reflect.Type{
		"ErrorResponse":                       reflect.TypeOf(ErrorResponse{}),
		"ErrorDetail":                         reflect.TypeOf(ErrorDetail{}),
		"SessionTokens":                       reflect.TypeOf(sessionTokens{}),
		"LoginRequest":                        reflect.TypeOf(loginRequest{}),
		"LoginResponse":                       reflect.TypeOf(loginResponse{}),
		"RefreshTokenRequest":                 reflect.TypeOf(refreshTokenRequest{}),
		"RefreshTokenResponse":                reflect.TypeOf(refreshTokenResponse{}),
		"LogoutRequest":                       reflect.TypeOf(logoutRequest{}),
		"LogoutResponse":                      reflect.TypeOf(logoutResponse{}),
		"ActivateDeviceRequest":               reflect.TypeOf(controlplane.ActivateDeviceInput{}),
		"DeviceRegistration":                  reflect.TypeOf(controlplane.DeviceRegistration{}),
		"ActivateDeviceResponse":              reflect.TypeOf(deviceEnvelope{}),
		"HeartbeatRequest":                    reflect.TypeOf(controlplane.HeartbeatInput{}),
		"HeartbeatResponse":                   reflect.TypeOf(heartbeatResponse{}),
		"ClientProfileResponse":               reflect.TypeOf(clientProfileResponse{}),
		"CreateModelPoolAccountRequest":       reflect.TypeOf(controlplane.CreateModelPoolAccountInput{}),
		"UpdateModelPoolAccountRequest":       reflect.TypeOf(controlplane.UpdateModelPoolAccountInput{}),
		"TestModelPoolAccountRequest":         reflect.TypeOf(controlplane.TestModelPoolAccountInput{}),
		"RotateModelPoolAccountSecretRequest": reflect.TypeOf(controlplane.RotateModelPoolAccountSecretInput{}),
		"ModelPoolConnectivityTestResponse":   reflect.TypeOf(modelPoolConnectivityTestResponse{}),
		"CreateModelLeaseRequest":             reflect.TypeOf(controlplane.CreateModelLeaseInput{}),
		"RenewModelLeaseRequest":              reflect.TypeOf(controlplane.RenewModelLeaseInput{}),
		"ReleaseModelLeaseRequest":            reflect.TypeOf(controlplane.ReleaseModelLeaseInput{}),
		"ModelLeaseResponse":                  reflect.TypeOf(modelLeaseResponse{}),
		"ReleaseModelLeaseResponse":           reflect.TypeOf(releaseModelLeaseResponse{}),
		"DirectLLMCallRecordRequest":          reflect.TypeOf(controlplane.CreateDirectLLMCallRecordInput{}),
		"DirectLLMCallRecordResponse":         reflect.TypeOf(directLLMCallRecordResponse{}),
		"Pagination":                          reflect.TypeOf(pagination{}),
		"UserEnvelope":                        reflect.TypeOf(userEnvelope{}),
		"UserAuthorizationSummaryResponse":    reflect.TypeOf(userAuthorizationSummaryResponse{}),
		"UserAuthorizationPolicyResponse":     reflect.TypeOf(userAuthorizationPolicyResponse{}),
		"DeviceEnvelope":                      reflect.TypeOf(deviceEnvelope{}),
		"UnbindDeviceResponse":                reflect.TypeOf(unbindDeviceResponse{}),
		"UserListResponse":                    reflect.TypeOf(userListResponse{}),
		"CreateUserRequest":                   reflect.TypeOf(controlplane.CreateUserInput{}),
		"UpdateUserRequest":                   reflect.TypeOf(controlplane.UpdateUserInput{}),
		"UpdateUserAuthorizationRequest":      reflect.TypeOf(controlplane.UpdateUserAuthorizationInput{}),
		"ResetUserPasswordRequest":            reflect.TypeOf(controlplane.ResetUserPasswordInput{}),
		"ChangeLocalAdminPasswordRequest":     reflect.TypeOf(controlplane.ChangeLocalAdminPasswordInput{}),
		"DeviceListResponse":                  reflect.TypeOf(deviceListResponse{}),
		"CreateActivationCodeRequest":         reflect.TypeOf(controlplane.CreateActivationCodeInput{}),
		"ActivationCode":                      reflect.TypeOf(controlplane.ActivationCode{}),
		"ActivationCodeEnvelope":              reflect.TypeOf(activationCodeEnvelope{}),
		"ActivationCodeListResponse":          reflect.TypeOf(activationCodeListResponse{}),
		"ModelPoolAccountEnvelope":            reflect.TypeOf(modelPoolAccountEnvelope{}),
		"ModelPoolAccountSummary":             reflect.TypeOf(controlplane.ModelPoolAccountSummary{}),
		"ModelPoolResponse":                   reflect.TypeOf(modelPoolResponse{}),
		"ModelUsageRecord":                    reflect.TypeOf(controlplane.ModelUsageRecord{}),
		"ModelUsageListResponse":              reflect.TypeOf(modelUsageListResponse{}),
		"ModelLease":                          reflect.TypeOf(controlplane.ModelLease{}),
		"ModelLeaseAdminSummary":              reflect.TypeOf(controlplane.ModelLeaseAdminSummary{}),
		"ModelLeaseListResponse":              reflect.TypeOf(modelLeaseListResponse{}),
		"ModelLeaseAdminDetail":               reflect.TypeOf(controlplane.ModelLeaseAdminDetail{}),
		"ModelLeaseAdminDetailResponse":       reflect.TypeOf(modelLeaseAdminDetailResponse{}),
		"AuditLog":                            reflect.TypeOf(controlplane.AuditLog{}),
		"AuditLogListResponse":                reflect.TypeOf(auditLogListResponse{}),
	}
	for schemaName, goType := range types {
		schemaNode, ok := document.Components["schemas"][schemaName]
		if !ok {
			t.Errorf("OpenAPI schema %s is missing", schemaName)
			continue
		}
		var schema struct {
			Required   []string             `yaml:"required"`
			Properties map[string]yaml.Node `yaml:"properties"`
		}
		if err := schemaNode.Decode(&schema); err != nil {
			t.Errorf("decode OpenAPI schema %s: %v", schemaName, err)
			continue
		}
		goFields := jsonFieldsForType(goType)
		openAPIFields := make(map[string]struct{}, len(schema.Properties))
		for property := range schema.Properties {
			openAPIFields[property] = struct{}{}
		}
		for field, metadata := range goFields {
			if _, ok := openAPIFields[field]; !ok {
				t.Errorf("%s: Go JSON field %q is missing from OpenAPI properties", schemaName, field)
			}
			if metadata.pointer && !metadata.omitEmpty && !openAPINodeAllowsNull(schema.Properties[field]) {
				t.Errorf("%s.%s: pointer field must be nullable in OpenAPI", schemaName, field)
			}
		}
		for field := range openAPIFields {
			if _, ok := goFields[field]; !ok {
				t.Errorf("%s: OpenAPI property %q is missing from Go JSON tags", schemaName, field)
			}
		}
		wantRequired := make(map[string]struct{})
		for _, field := range schema.Required {
			wantRequired[field] = struct{}{}
		}
		gotRequired := make(map[string]struct{})
		for field, metadata := range goFields {
			if !metadata.omitEmpty && !metadata.pointer {
				gotRequired[field] = struct{}{}
			}
		}
		if !reflect.DeepEqual(wantRequired, gotRequired) {
			t.Errorf("%s: required fields drift: OpenAPI=%v Go=%v", schemaName, sortedSet(wantRequired), sortedSet(gotRequired))
		}
	}
}

func TestOpenAPIErrorResponseExamplesAndRuntimeStatusMatrix(t *testing.T) {
	document := loadOpenAPIContract(t)
	_, sourceFile, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("runtime.Caller() failed")
	}
	errorCodeDocumentPath := filepath.Clean(filepath.Join(filepath.Dir(sourceFile), "..", "..", "..", "接口契约", "错误码.md"))
	errorCodeDocument, err := os.ReadFile(errorCodeDocumentPath)
	if err != nil {
		t.Fatalf("read error code document %q: %v", errorCodeDocumentPath, err)
	}
	documentedCodes := make(map[string]struct{})
	for _, line := range strings.Split(string(errorCodeDocument), "\n") {
		if matches := errorCodeRow.FindStringSubmatch(strings.TrimSpace(line)); len(matches) == 2 {
			documentedCodes[matches[1]] = struct{}{}
		}
	}
	responseComponents := []struct {
		status int
		name   string
	}{
		{status: http.StatusBadRequest, name: "BadRequest"},
		{status: http.StatusUnauthorized, name: "Unauthorized"},
		{status: http.StatusForbidden, name: "Forbidden"},
		{status: http.StatusNotFound, name: "NotFound"},
		{status: http.StatusConflict, name: "Conflict"},
		{status: http.StatusTooManyRequests, name: "TooManyRequests"},
		{status: http.StatusBadGateway, name: "BadGateway"},
		{status: http.StatusServiceUnavailable, name: "ServiceUnavailable"},
		{status: http.StatusGatewayTimeout, name: "GatewayTimeout"},
		{status: http.StatusMethodNotAllowed, name: "MethodNotAllowed"},
		{status: http.StatusRequestTimeout, name: "RequestTimeout"},
		{status: http.StatusInternalServerError, name: "InternalServerError"},
	}
	for _, response := range responseComponents {
		node, ok := document.Components["responses"][response.name]
		if !ok {
			t.Errorf("OpenAPI response component %s is missing", response.name)
			continue
		}
		var contract struct {
			Content map[string]struct {
				Schema   yaml.Node            `yaml:"schema"`
				Examples map[string]yaml.Node `yaml:"examples"`
			} `yaml:"content"`
		}
		if err := node.Decode(&contract); err != nil {
			t.Errorf("decode OpenAPI response %s: %v", response.name, err)
			continue
		}
		content, ok := contract.Content["application/json"]
		if !ok {
			t.Errorf("OpenAPI response %s has no application/json content", response.name)
			continue
		}
		if ref := localReferenceName(content.Schema); ref != "ErrorResponse" {
			t.Errorf("OpenAPI response %s schema ref = %q, want ErrorResponse", response.name, ref)
		}
		if len(content.Examples) == 0 {
			t.Errorf("OpenAPI response %s has no JSON example", response.name)
			continue
		}
		for exampleName, exampleNode := range content.Examples {
			var example struct {
				Value map[string]any `yaml:"value"`
			}
			if err := exampleNode.Decode(&example); err != nil {
				t.Errorf("decode OpenAPI response %s example %s: %v", response.name, exampleName, err)
				continue
			}
			for _, field := range []string{"code", "message", "request_id"} {
				value, exists := example.Value[field]
				if !exists || strings.TrimSpace(fmt.Sprint(value)) == "" {
					t.Errorf("OpenAPI response %s example %s missing non-empty %s", response.name, exampleName, field)
				}
			}
			if code, ok := example.Value["code"].(string); ok {
				if _, documented := documentedCodes[code]; !documented {
					t.Errorf("OpenAPI response %s example %s uses undocumented error code %q", response.name, exampleName, code)
				}
			}
		}
	}

	runRoute := func(t *testing.T, method, path string, body any, token, idempotencyKey string) *httptest.ResponseRecorder {
		t.Helper()
		handler := newTestRouter(t)
		if token == "login" {
			token = loginForTest(t, handler)
		}
		return doJSON(t, handler, method, path, body, token, idempotencyKey)
	}

	runtimeCases := []struct {
		name   string
		status int
		code   string
		run    func(*testing.T) *httptest.ResponseRecorder
	}{
		{
			name:   "bad request",
			status: http.StatusBadRequest,
			code:   "INVALID_REQUEST",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				return runRoute(t, http.MethodPost, "/api/v1/auth/login", nil, "", "")
			},
		},
		{
			name:   "unauthorized",
			status: http.StatusUnauthorized,
			code:   "UNAUTHENTICATED",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				return runRoute(t, http.MethodGet, "/api/v1/admin/users?page_size=1", nil, "", "")
			},
		},
		{
			name:   "forbidden",
			status: http.StatusForbidden,
			code:   "DEVICE_BINDING_REQUIRED",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				return runRoute(t, http.MethodPost, "/api/v1/client/model-leases", map[string]any{"provider": "openai-compatible", "model": "contract-model"}, "login", "contract-forbidden")
			},
		},
		{
			name:   "not found",
			status: http.StatusNotFound,
			code:   "NOT_FOUND",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				return runRoute(t, http.MethodGet, "/contract/missing", nil, "", "")
			},
		},
		{
			name:   "conflict",
			status: http.StatusConflict,
			code:   "IDEMPOTENCY_CONFLICT",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				handler := newTestRouter(t)
				token := loginForTest(t, handler)
				first := doJSON(t, handler, http.MethodPost, "/api/v1/admin/users", map[string]any{"username": "contract_user", "password": "contract-password", "role": "user"}, token, "contract-conflict")
				if first.Code != http.StatusCreated {
					t.Fatalf("conflict setup status = %d, body=%s", first.Code, first.Body.String())
				}
				return doJSON(t, handler, http.MethodPost, "/api/v1/admin/users", map[string]any{"username": "contract_user_2", "password": "contract-password", "role": "user"}, token, "contract-conflict")
			},
		},
		{
			name:   "too many requests",
			status: http.StatusTooManyRequests,
			code:   "RATE_LIMITED",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				handler := newTestRouter(t)
				var last *httptest.ResponseRecorder
				for attempt := 0; attempt < 6; attempt++ {
					req := httptest.NewRequest(http.MethodPost, "/api/v1/auth/login", strings.NewReader(`{"username":"admin","password":"wrong-password"}`))
					req.RemoteAddr = "198.51.100.42:9000"
					req.Header.Set("Content-Type", "application/json")
					last = httptest.NewRecorder()
					handler.ServeHTTP(last, req)
				}
				return last
			},
		},
		{
			name:   "bad gateway mapper",
			status: http.StatusBadGateway,
			code:   "MODEL_POOL_SECRET_VALIDATION_FAILED",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				return runErrorMapper(t, controlplane.ErrModelPoolSecretValidation)
			},
		},
		{
			name:   "service unavailable mapper",
			status: http.StatusServiceUnavailable,
			code:   "SECRET_STORE_UNAVAILABLE",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				return runErrorMapper(t, controlplane.ErrSecretStoreUnavailable)
			},
		},
		{
			name:   "unknown commit mapper",
			status: http.StatusServiceUnavailable,
			code:   "COMMIT_OUTCOME_UNKNOWN",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				return runErrorMapper(t, fmt.Errorf("commit response lost: %w", store.ErrCommitOutcomeUnknown))
			},
		},
		{
			name:   "gateway timeout mapper",
			status: http.StatusGatewayTimeout,
			code:   "REQUEST_TIMEOUT",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				return runErrorMapper(t, context.DeadlineExceeded)
			},
		},
		{
			name:   "method not allowed",
			status: http.StatusMethodNotAllowed,
			code:   "METHOD_NOT_ALLOWED",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				return runRoute(t, http.MethodPost, "/api/v1/health", nil, "", "")
			},
		},
		{
			name:   "request canceled mapper",
			status: http.StatusRequestTimeout,
			code:   "REQUEST_CANCELED",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				return runErrorMapper(t, context.Canceled)
			},
		},
		{
			name:   "internal error mapper",
			status: http.StatusInternalServerError,
			code:   "INTERNAL_ERROR",
			run: func(t *testing.T) *httptest.ResponseRecorder {
				return runErrorMapper(t, errors.New("contract internal error"))
			},
		},
	}

	for _, test := range runtimeCases {
		t.Run(test.name, func(t *testing.T) {
			recorder := test.run(t)
			if recorder.Code != test.status {
				t.Fatalf("runtime status = %d, want %d; body=%s", recorder.Code, test.status, recorder.Body.String())
			}
			if !strings.Contains(recorder.Header().Get("Content-Type"), "application/json") {
				t.Fatalf("runtime Content-Type = %q", recorder.Header().Get("Content-Type"))
			}
			var payload ErrorResponse
			if err := json.Unmarshal(recorder.Body.Bytes(), &payload); err != nil {
				t.Fatalf("runtime error JSON: %v; body=%s", err, recorder.Body.String())
			}
			if payload.Code != test.code || payload.Message == "" || payload.RequestID == "" {
				t.Fatalf("runtime error payload = %+v, want code %s", payload, test.code)
			}
		})
	}
}

func localReferenceName(node yaml.Node) string {
	if node.Kind != yaml.MappingNode {
		return ""
	}
	for index := 0; index+1 < len(node.Content); index += 2 {
		if node.Content[index].Value == "$ref" {
			ref := node.Content[index+1].Value
			ref = strings.TrimPrefix(ref, "#/components/schemas/")
			return strings.TrimPrefix(ref, "#/components/responses/")
		}
	}
	return ""
}

func runErrorMapper(t *testing.T, err error) *httptest.ResponseRecorder {
	t.Helper()
	handler := requestIDMiddleware(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		writeAppError(w, r, err)
	}))
	recorder := httptest.NewRecorder()
	handler.ServeHTTP(recorder, httptest.NewRequest(http.MethodGet, "/contract/error", nil))
	return recorder
}

type jsonFieldMetadata struct {
	omitEmpty bool
	pointer   bool
}

func jsonFieldsForType(goType reflect.Type) map[string]jsonFieldMetadata {
	fields := make(map[string]jsonFieldMetadata)
	collectJSONFields(goType, fields)
	return fields
}

func collectJSONFields(goType reflect.Type, fields map[string]jsonFieldMetadata) {
	if goType.Kind() == reflect.Pointer {
		goType = goType.Elem()
	}
	if goType.Kind() != reflect.Struct {
		return
	}
	for index := 0; index < goType.NumField(); index++ {
		field := goType.Field(index)
		if field.Anonymous {
			collectJSONFields(field.Type, fields)
			continue
		}
		tag := field.Tag.Get("json")
		if tag == "" {
			continue
		}
		parts := strings.Split(tag, ",")
		if parts[0] == "-" || parts[0] == "" {
			continue
		}
		fields[parts[0]] = jsonFieldMetadata{
			omitEmpty: slices.Contains(parts[1:], "omitempty"),
			pointer:   field.Type.Kind() == reflect.Pointer,
		}
	}
}

func openAPINodeAllowsNull(node yaml.Node) bool {
	if node.Kind == 0 {
		return false
	}
	var property struct {
		Type any `yaml:"type"`
	}
	if node.Decode(&property) != nil {
		return false
	}
	if values, ok := property.Type.([]any); ok {
		for _, value := range values {
			if value == "null" {
				return true
			}
		}
	}
	return false
}

func sortedSet(values map[string]struct{}) []string {
	items := make([]string, 0, len(values))
	for value := range values {
		items = append(items, value)
	}
	sort.Strings(items)
	return items
}

func TestGoErrorCodesMatchErrorCodeDocument(t *testing.T) {
	_, sourceFile, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("runtime.Caller() failed")
	}
	internalRoot := filepath.Clean(filepath.Join(filepath.Dir(sourceFile), ".."))
	goCodes := make(map[string]struct{})
	goStatuses := make(map[string]int)
	if err := filepath.Walk(internalRoot, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		if info.IsDir() || filepath.Ext(path) != ".go" || strings.HasSuffix(info.Name(), "_test.go") {
			return nil
		}
		file, err := parser.ParseFile(token.NewFileSet(), path, nil, 0)
		if err != nil {
			return fmt.Errorf("parse %s: %w", path, err)
		}
		ast.Inspect(file, func(node ast.Node) bool {
			call, ok := node.(*ast.CallExpr)
			if !ok {
				return true
			}
			codeIndex := -1
			switch function := call.Fun.(type) {
			case *ast.Ident:
				switch function.Name {
				case "NewError":
					codeIndex = 1
				case "writeError":
					codeIndex = 3
				}
			case *ast.SelectorExpr:
				if function.Sel.Name == "NewError" {
					codeIndex = 1
				}
			}
			if codeIndex < 0 || len(call.Args) <= codeIndex {
				return true
			}
			literal, ok := call.Args[codeIndex].(*ast.BasicLit)
			if !ok || literal.Kind != token.STRING {
				return true
			}
			code, err := strconv.Unquote(literal.Value)
			if err == nil {
				goCodes[code] = struct{}{}
				if function, ok := call.Fun.(*ast.Ident); ok && function.Name == "NewError" {
					if status, ok := httpStatusValue(call.Args[0]); ok {
						if previous, exists := goStatuses[code]; exists && previous != status {
							t.Fatalf("Go error code %s has inconsistent HTTP statuses %d and %d", code, previous, status)
						}
						goStatuses[code] = status
					}
				}
			}
			return true
		})
		return nil
	}); err != nil {
		t.Fatalf("scan Go error codes: %v", err)
	}

	documentPath := filepath.Clean(filepath.Join(filepath.Dir(sourceFile), "..", "..", "..", "接口契约", "错误码.md"))
	document, err := os.ReadFile(documentPath)
	if err != nil {
		t.Fatalf("read error code document %q: %v", documentPath, err)
	}
	docCodes := make(map[string]struct{})
	docStatuses := make(map[string]int)
	for _, line := range strings.Split(string(document), "\n") {
		matches := errorCodeRow.FindStringSubmatch(strings.TrimSpace(line))
		if len(matches) == 2 {
			docCodes[matches[1]] = struct{}{}
		}
		statusMatches := errorCodeStatusRow.FindStringSubmatch(strings.TrimSpace(line))
		if len(statusMatches) == 3 {
			status, parseErr := strconv.Atoi(statusMatches[2])
			if parseErr != nil {
				t.Fatalf("parse documented HTTP status for %s: %v", statusMatches[1], parseErr)
			}
			docStatuses[statusMatches[1]] = status
		}
	}

	var missing []string
	for code := range goCodes {
		if _, ok := docCodes[code]; !ok {
			missing = append(missing, code)
		}
	}
	sort.Strings(missing)
	if len(missing) > 0 {
		t.Fatalf("error code drift: missing in document=%v (client/future-only documented codes are allowed)", missing)
	}
	var mismatched []string
	for code, wantStatus := range goStatuses {
		if gotStatus, ok := docStatuses[code]; !ok {
			mismatched = append(mismatched, fmt.Sprintf("%s missing HTTP status (want %d)", code, wantStatus))
		} else if gotStatus != wantStatus {
			mismatched = append(mismatched, fmt.Sprintf("%s documented %d, Go %d", code, gotStatus, wantStatus))
		}
	}
	sort.Strings(mismatched)
	if len(mismatched) > 0 {
		t.Fatalf("error code HTTP status drift: %s", strings.Join(mismatched, "; "))
	}
}

func httpStatusValue(expression ast.Expr) (int, bool) {
	selector, ok := expression.(*ast.SelectorExpr)
	if !ok {
		literal, ok := expression.(*ast.BasicLit)
		if !ok || literal.Kind != token.INT {
			return 0, false
		}
		value, err := strconv.Atoi(literal.Value)
		return value, err == nil
	}
	packageName, ok := selector.X.(*ast.Ident)
	if !ok || packageName.Name != "http" {
		return 0, false
	}
	statuses := map[string]int{
		"StatusBadRequest":          http.StatusBadRequest,
		"StatusUnauthorized":        http.StatusUnauthorized,
		"StatusForbidden":           http.StatusForbidden,
		"StatusNotFound":            http.StatusNotFound,
		"StatusConflict":            http.StatusConflict,
		"StatusTooManyRequests":     http.StatusTooManyRequests,
		"StatusBadGateway":          http.StatusBadGateway,
		"StatusServiceUnavailable":  http.StatusServiceUnavailable,
		"StatusInternalServerError": http.StatusInternalServerError,
	}
	value, ok := statuses[selector.Sel.Name]
	return value, ok
}
