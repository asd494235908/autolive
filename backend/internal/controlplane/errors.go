package controlplane

import (
	"errors"
	"net/http"
)

type Error struct {
	Code    string
	Message string
	Status  int
}

func (e *Error) Error() string {
	return e.Code + ": " + e.Message
}

func NewError(status int, code, message string) *Error {
	return &Error{
		Code:    code,
		Message: message,
		Status:  status,
	}
}

func IsErrorCode(err error, code string) bool {
	var appErr *Error
	return errors.As(err, &appErr) && appErr.Code == code
}

var (
	ErrUnauthenticated                 = NewError(http.StatusUnauthorized, "UNAUTHENTICATED", "用户名或密码错误")
	ErrForbidden                       = NewError(http.StatusForbidden, "FORBIDDEN", "没有访问该资源的权限")
	ErrInvalidRequest                  = NewError(http.StatusBadRequest, "INVALID_ARGUMENT", "请求格式或参数无效")
	ErrIdempotencyKeyRequired          = NewError(http.StatusBadRequest, "INVALID_IDEMPOTENCY_KEY", "写操作必须提供 Idempotency-Key")
	ErrIdempotencyConflict             = NewError(http.StatusConflict, "IDEMPOTENCY_CONFLICT", "幂等键对应的请求语义冲突")
	ErrActivationCodeAlreadyUsed       = NewError(http.StatusConflict, "ACTIVATION_CODE_USED", "激活码已被核销")
	ErrActivationCodeExpired           = NewError(http.StatusForbidden, "ACTIVATION_CODE_EXPIRED", "激活码已过期")
	ErrActivationCodeRevoked           = NewError(http.StatusForbidden, "ACTIVATION_CODE_REVOKED", "激活码已失效")
	ErrActivationCodeStateConflict     = NewError(http.StatusConflict, "ACTIVATION_CODE_STATE_CONFLICT", "激活码当前状态不允许作废")
	ErrAccountActivationRequired       = NewError(http.StatusForbidden, "ACCOUNT_ACTIVATION_REQUIRED", "账号尚未绑定有效激活授权")
	ErrAccountActivationExpired        = NewError(http.StatusForbidden, "ACCOUNT_ACTIVATION_EXPIRED", "账号激活授权已过期")
	ErrDeviceLimitExceeded             = NewError(http.StatusConflict, "DEVICE_LIMIT_EXCEEDED", "账号允许登录的设备数量已达上限")
	ErrDeviceDisabled                  = NewError(http.StatusForbidden, "DEVICE_DISABLED", "设备已被禁用")
	ErrDeviceBindingRequired           = NewError(http.StatusForbidden, "DEVICE_BINDING_REQUIRED", "当前会话尚未绑定已激活设备")
	ErrDeviceNotFound                  = NewError(http.StatusNotFound, "DEVICE_NOT_FOUND", "设备不存在")
	ErrDeviceBindingConflict           = NewError(http.StatusConflict, "DEVICE_BINDING_CONFLICT", "设备已绑定其他用户或已存在")
	ErrUserDisabled                    = NewError(http.StatusForbidden, "USER_DISABLED", "用户已被禁用")
	ErrUserNotFound                    = NewError(http.StatusNotFound, "USER_NOT_FOUND", "用户不存在")
	ErrUserModelNotAuthorized          = NewError(http.StatusForbidden, "USER_MODEL_NOT_AUTHORIZED", "用户未获授权使用该模型")
	ErrUserRecordedQuotaExceeded       = NewError(http.StatusTooManyRequests, "USER_RECORDED_QUOTA_EXCEEDED", "用户已达到服务端记录用量上限")
	ErrCannotDisableLocalAdmin         = NewError(http.StatusConflict, "CANNOT_DISABLE_LOCAL_ADMIN", "开发阶段内置管理员不能被禁用")
	ErrCannotModifyLocalAdmin          = NewError(http.StatusConflict, "CANNOT_MODIFY_LOCAL_ADMIN", "开发阶段内置管理员不能通过用户管理接口修改")
	ErrLocalAdminRequired              = NewError(http.StatusForbidden, "LOCAL_ADMIN_REQUIRED", "只有本地管理员可以执行该操作")
	ErrLastActiveAdmin                 = NewError(http.StatusConflict, "LAST_ACTIVE_ADMIN", "至少需要保留一个有效管理员")
	ErrAdminPermissionUnknown          = NewError(http.StatusBadRequest, "ADMIN_PERMISSION_UNKNOWN", "存在未登记的管理员权限代码")
	ErrAdminPermissionDenied           = NewError(http.StatusForbidden, "ADMIN_PERMISSION_DENIED", "当前管理员权限不足")
	ErrAdminRoleNotFound               = NewError(http.StatusNotFound, "ADMIN_ROLE_NOT_FOUND", "管理员角色不存在")
	ErrAdminBuiltInRoleImmutable       = NewError(http.StatusConflict, "ADMIN_BUILTIN_ROLE_IMMUTABLE", "内置管理员角色不允许修改或删除")
	ErrAdminRoleAssigned               = NewError(http.StatusConflict, "ADMIN_ROLE_ASSIGNED", "管理员角色仍被用户绑定")
	ErrAdminRoleDelegationForbidden    = NewError(http.StatusForbidden, "ADMIN_ROLE_DELEGATION_FORBIDDEN", "当前管理员不能委派超出自身权限或范围的角色")
	ErrAdminLastSuperAdminProtected    = NewError(http.StatusConflict, "ADMIN_LAST_SUPER_ADMIN_PROTECTED", "至少需要保留一个有效的超级管理员")
	ErrAdminProductScopeMismatch       = NewError(http.StatusForbidden, "ADMIN_PRODUCT_SCOPE_MISMATCH", "管理员角色分配与产品范围不匹配")
	ErrActivationCodeNotFound          = NewError(http.StatusNotFound, "ACTIVATION_CODE_NOT_FOUND", "激活码不存在")
	ErrUsernameAlreadyExists           = NewError(http.StatusConflict, "USERNAME_ALREADY_EXISTS", "用户名已存在")
	ErrModelPoolUnavailable            = NewError(http.StatusTooManyRequests, "MODEL_POOL_UNAVAILABLE", "当前没有可分配的模型账号")
	ErrModelPoolAccountNotFound        = NewError(http.StatusNotFound, "MODEL_POOL_ACCOUNT_NOT_FOUND", "模型号池账号不存在")
	ErrModelPoolAccountInUse           = NewError(http.StatusConflict, "MODEL_POOL_ACCOUNT_IN_USE", "模型号池账号仍有运行中的租约")
	ErrModelPoolSecretValidation       = NewError(http.StatusBadGateway, "MODEL_POOL_SECRET_VALIDATION_FAILED", "新模型密钥连通性校验失败")
	ErrModelPoolSecretRotationConflict = NewError(http.StatusConflict, "MODEL_POOL_SECRET_ROTATION_CONFLICT", "模型账号密钥已被其他轮换请求更新")
	ErrModelPoolConcurrencyConflict    = NewError(http.StatusConflict, "MODEL_POOL_CONCURRENCY_CONFLICT", "并发上限不能低于当前活动租约数")
	ErrModelLeaseNotFound              = NewError(http.StatusNotFound, "MODEL_LEASE_NOT_FOUND", "模型租约不存在")
	ErrModelLeaseStateConflict         = NewError(http.StatusConflict, "MODEL_LEASE_STATE_CONFLICT", "模型租约状态不允许当前操作")
	ErrSecretStoreUnavailable          = NewError(http.StatusServiceUnavailable, "SECRET_STORE_UNAVAILABLE", "服务端密钥存储暂不可用")
	ErrCommitOutcomeUnknown            = NewError(http.StatusServiceUnavailable, "COMMIT_OUTCOME_UNKNOWN", "服务端无法确认本次写入是否已提交")
)
