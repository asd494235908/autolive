package controlplane

import "net/http"

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
	appErr, ok := err.(*Error)
	return ok && appErr.Code == code
}

var (
	ErrUnauthenticated              = NewError(http.StatusUnauthorized, "UNAUTHENTICATED", "用户名或密码错误")
	ErrForbidden                    = NewError(http.StatusForbidden, "FORBIDDEN", "没有访问该资源的权限")
	ErrInvalidRequest               = NewError(http.StatusBadRequest, "INVALID_ARGUMENT", "请求格式或参数无效")
	ErrIdempotencyKeyRequired       = NewError(http.StatusBadRequest, "INVALID_IDEMPOTENCY_KEY", "写操作必须提供 Idempotency-Key")
	ErrIdempotencyConflict          = NewError(http.StatusConflict, "IDEMPOTENCY_CONFLICT", "幂等键对应的请求语义冲突")
	ErrActivationCodeAlreadyUsed    = NewError(http.StatusConflict, "ACTIVATION_CODE_USED", "激活码已被核销")
	ErrActivationCodeExpired        = NewError(http.StatusForbidden, "ACTIVATION_CODE_EXPIRED", "激活码已过期")
	ErrActivationCodeRevoked        = NewError(http.StatusForbidden, "ACTIVATION_CODE_REVOKED", "激活码已失效")
	ErrActivationCodeStateConflict  = NewError(http.StatusConflict, "ACTIVATION_CODE_STATE_CONFLICT", "激活码当前状态不允许作废")
	ErrDeviceDisabled               = NewError(http.StatusForbidden, "DEVICE_DISABLED", "设备已被禁用")
	ErrDeviceBindingRequired        = NewError(http.StatusForbidden, "DEVICE_BINDING_REQUIRED", "当前会话尚未绑定已激活设备")
	ErrDeviceNotFound               = NewError(http.StatusNotFound, "DEVICE_NOT_FOUND", "设备不存在")
	ErrDeviceBindingConflict        = NewError(http.StatusConflict, "DEVICE_BINDING_CONFLICT", "设备已绑定其他用户或已存在")
	ErrUserDisabled                 = NewError(http.StatusForbidden, "USER_DISABLED", "用户已被禁用")
	ErrUserNotFound                 = NewError(http.StatusNotFound, "USER_NOT_FOUND", "用户不存在")
	ErrCannotDisableLocalAdmin      = NewError(http.StatusConflict, "CANNOT_DISABLE_LOCAL_ADMIN", "开发阶段内置管理员不能被禁用")
	ErrActivationCodeNotFound       = NewError(http.StatusNotFound, "ACTIVATION_CODE_NOT_FOUND", "激活码不存在")
	ErrUsernameAlreadyExists        = NewError(http.StatusConflict, "USERNAME_ALREADY_EXISTS", "用户名已存在")
	ErrModelPoolUnavailable         = NewError(http.StatusTooManyRequests, "MODEL_POOL_UNAVAILABLE", "当前没有可分配的模型账号")
	ErrModelPoolAccountNotFound     = NewError(http.StatusNotFound, "MODEL_POOL_ACCOUNT_NOT_FOUND", "模型号池账号不存在")
	ErrModelPoolAccountInUse        = NewError(http.StatusConflict, "MODEL_POOL_ACCOUNT_IN_USE", "模型号池账号仍有运行中的租约")
	ErrModelPoolConcurrencyConflict = NewError(http.StatusConflict, "MODEL_POOL_CONCURRENCY_CONFLICT", "并发上限不能低于当前活动租约数")
	ErrModelLeaseNotFound           = NewError(http.StatusNotFound, "MODEL_LEASE_NOT_FOUND", "模型租约不存在")
	ErrModelLeaseStateConflict      = NewError(http.StatusConflict, "MODEL_LEASE_STATE_CONFLICT", "模型租约状态不允许当前操作")
	ErrSecretStoreUnavailable       = NewError(http.StatusServiceUnavailable, "SECRET_STORE_UNAVAILABLE", "服务端密钥存储暂不可用")
)
