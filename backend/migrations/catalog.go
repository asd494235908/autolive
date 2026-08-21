package migrations

import "embed"

const LatestVersion = 22

// FS 是发布时使用的只读迁移目录；迁移执行不由 API 服务启动过程隐式触发。
//
//go:embed *.up.sql
var FS embed.FS
