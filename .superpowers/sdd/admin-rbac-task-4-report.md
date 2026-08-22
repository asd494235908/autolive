# Admin RBAC Task 4 报告

## 改动文件

- `backend/internal/service/admin_rbac.go`
- `backend/internal/service/admin_rbac_test.go`

## 实际执行命令与结果

1. `cd backend && go test ./internal/service -run 'AdminRBAC|RBAC' -count=1`
   - 结果：PASS
2. `cd backend && gofmt -w internal/service/admin_rbac.go internal/service/admin_rbac_test.go`
   - 结果：PASS
3. `git diff --check`
   - 结果：PASS

## 完成内容

- 新增 RBAC 服务层最小实现，提供：
  - `GetAdminAuthorization`
  - 权限目录读取
  - 角色读取 / 创建 / 更新 / 删除
  - 用户角色读取 / 按产品或全局范围替换
- 服务层默认 fail-closed：
  - 不会把任意 `users.role=admin` 自动视为全局超级管理员
  - 仅 `usr_local_admin` 兼容规则或持久化 `super_admin` 绑定生效
- 委派规则已在服务层收敛：
  - 普通管理员只能操作自己当前产品
  - 普通管理员不能分配或撤销全局 `super_admin`
  - 目标角色权限必须是操作者即时权限子集
  - 产品范围替换只覆盖当前作用域，保留其他产品已有绑定
- Memory 路径补上了最后超级管理员保护
- 写操作通过现有审计入口记录目标、结果和错误码，不写入请求正文

## 测试覆盖点

- 权限并集
- 产品不匹配
- 全局 `super_admin`
- 停用即时失效
- 普通管理员不能授予自身没有的权限
- 目标角色权限必须是操作者权限子集
- 普通角色不能全局
- 内建角色不可改
- 绑定角色不可删
- 最后超级管理员 / 本地管理员兼容保护
- 幂等重放 / 冲突
- 审计脱敏

## 未验证项

- 未实现也未验证 HTTP/OpenAPI/React 集成（按 Task 5/6 边界留待后续）
- 未运行桌面端验证（按要求跳过）
- 未运行更大范围的 `go test ./...`、`go vet` 或 PostgreSQL 集成验证

## 剩余风险

- 当前 Task 4 仅覆盖 service 边界；真正的 API 权限矩阵、HTTP 错误映射与 OpenAPI 契约还需要 Task 5 衔接验证
- 审计在 RBAC service 中通过现有入口单独记录，未与 RBAC repository 写事务合并

## 2026-08-22 Task 4 审查 Important 修复

### 本次改动文件

- `backend/internal/service/admin_rbac.go`
- `backend/internal/service/admin_rbac_test.go`
- `backend/internal/store/admin_rbac_repository.go`
- `backend/internal/store/admin_rbac_repository_test.go`
- `backend/internal/store/memory_admin_rbac.go`
- `backend/internal/store/postgres_admin_rbac_repository.go`
- `backend/internal/store/postgres_admin_rbac_repository_test.go`

### 修复内容

1. 审计事务边界
   - 为 `AdminRoleWriteRecord`、`AdminRoleDeleteRecord`、`UserAdminRoleReplaceRecord` 增加 `Audit` 字段。
   - service 成功路径预构建脱敏 success audit 并传给 repository；service 只在失败/校验失败时补 failure audit。
   - Memory RBAC mutation 在同一次 `Run` 内写入脱敏 success audit，并按 `RequestID` 去重重放审计。
   - Postgres RBAC Create/Update/Delete/Replace 在业务 mutation 同一事务里调用现有 `enqueueAuditOutboxTx`；成功审计入队失败会直接回滚业务事务。

2. scoped `ListAdminRoles`
   - 非 global `super_admin` 的 service 返回现在只保留 actor 当前 product 的普通角色。
   - repository 仍保留“查询时包含全局 `super_admin`”语义，service 负责按范围过滤，和 `GetAdminRole` 的范围判断保持一致。

3. `usr_local_admin` 返回一致
   - Postgres `ReplaceUserAdminRoles` 成功路径返回值改为 `withCompatibilityLocalSuperAdmin(user, normalizedAssignments)`。
   - 现在 success / replay / list / Memory 的 `usr_local_admin` 兼容 `super_admin` 语义一致，不会出现首次返回与后续鉴权不一致。

### 本次实际执行命令与结果

1. `gofmt -w backend/internal/store/admin_rbac_repository.go backend/internal/store/memory_admin_rbac.go backend/internal/store/postgres_admin_rbac_repository.go backend/internal/store/admin_rbac_repository_test.go backend/internal/store/postgres_admin_rbac_repository_test.go backend/internal/service/admin_rbac.go backend/internal/service/admin_rbac_test.go`
   - 结果：PASS
2. `cd backend && go test ./internal/service -run 'AdminRBAC|RBAC' -count=1`
   - 结果：PASS
3. `cd backend && go test ./internal/store -run 'AdminRBAC|RBAC' -count=1`
   - 结果：PASS
4. `git diff --check`
   - 结果：PASS

### 本次新增/补强验证

- service scoped 角色列表不会向普通产品管理员暴露全局 `super_admin`
- Memory success audit 与 RBAC 变更同次 mutation 写入，重放按 `RequestID` 去重
- Memory 审计日志不包含角色名称/权限列表等请求正文信息
- Postgres 角色 Create/Update/Delete success audit 进入现有 outbox 事务
- Postgres `ReplaceUserAdminRoles` replay 会提交 success audit 出队请求
- Postgres `usr_local_admin` 在 success replace 响应里保留兼容 `super_admin`

### 更新后的剩余风险

- 未运行 PostgreSQL 真库集成场景，本次只覆盖 sqlmock/Memory 的聚焦验证
- 未实现也未验证 HTTP/OpenAPI/React 接线，仍留给 Task 5/6
- 未运行桌面端验证、未跑全量 `go test ./...`
