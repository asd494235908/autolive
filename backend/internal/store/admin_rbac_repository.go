package store

import (
	"context"

	"autoLive/backend/internal/controlplane"
)

type AdminRoleRecord struct {
	Code        string                        `json:"code"`
	Product     controlplane.ProductCode      `json:"product"`
	Name        string                        `json:"name"`
	BuiltIn     bool                          `json:"built_in"`
	Permissions []controlplane.PermissionCode `json:"permissions"`
}

type AdminRoleWriteRecord struct {
	Scope          string
	IdempotencyKey string
	Fingerprint    string
	Audit          controlplane.AuditLogInput
	Role           AdminRoleRecord
}

type AdminRoleDeleteRecord struct {
	Scope          string
	IdempotencyKey string
	Fingerprint    string
	Audit          controlplane.AuditLogInput
	Code           string
}

type UserAdminRoleReplaceRecord struct {
	Scope          string
	IdempotencyKey string
	Fingerprint    string
	Audit          controlplane.AuditLogInput
	UserID         string
	Assignments    []controlplane.AdminRoleAssignment
}

type AdminRBACRepository interface {
	GetAdminAuthorization(ctx context.Context, userID string, product controlplane.ProductCode) (controlplane.AdminAuthorization, error)
	ListAdminPermissions(ctx context.Context) ([]controlplane.PermissionCode, error)
	ListAdminRoles(ctx context.Context, product controlplane.ProductCode) ([]AdminRoleRecord, error)
	GetAdminRole(ctx context.Context, code string) (AdminRoleRecord, error)
	CreateAdminRole(ctx context.Context, record AdminRoleWriteRecord) (AdminRoleRecord, error)
	UpdateAdminRole(ctx context.Context, record AdminRoleWriteRecord) (AdminRoleRecord, error)
	DeleteAdminRole(ctx context.Context, record AdminRoleDeleteRecord) error
	ListUserAdminRoles(ctx context.Context, userID string, product controlplane.ProductCode) ([]controlplane.AdminRoleAssignment, error)
	ReplaceUserAdminRoles(ctx context.Context, record UserAdminRoleReplaceRecord) ([]controlplane.AdminRoleAssignment, error)
}
