package store

import (
	"context"
	"time"
)

// StateOperation 是控制面一次受保护读写操作的边界。
// MemoryStore 用互斥锁实现；生产实现必须映射为数据库事务，不得静默回退到内存。
type StateOperation func(state *State) error

// Repository 是 ControlPlane 的可替换持久化边界。
// State 目前保留领域层的统一快照形状，后续 PostgreSQL 适配器负责把它映射到迁移表，
// Secret Store 只负责密钥字段，不得通过该接口把明文密钥返回给 API 层。
type Repository interface {
	Now() time.Time
	Run(ctx context.Context, fn StateOperation) error
}
