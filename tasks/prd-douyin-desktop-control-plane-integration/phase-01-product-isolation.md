# Phase 1：产品硬隔离与管理范围

Parent PRD：[PRD：douyin-desktop 接入 autoLive 多产品控制面](../prd-douyin-desktop-control-plane-integration.md)
状态：Phase 1 基线已实现；终态收紧与最终验收待后续阶段
最后更新：2026-08-22

> 本轮已完成产品值对象、迁移 0023、产品绑定会话、设备/激活码/Profile/租约/用量/审计的已接入服务端产品边界，以及管理端列表/设备详情的服务端授权收窄；全局用户授权摘要和用户管理写操作在产品角色生命周期完成前仅内建本地管理员可用。当前仍保留兼容快照与旧设备唯一键；固定权限目录、产品角色生命周期、React 产品权限壳、Profile 6h/24h 离线签名、模型委托凭证、不可变 OpenAPI 制品和跨仓库 E2E 不属于本阶段已完成范围。

## 目标

建立 `products`、`user_products`、产品会话和产品范围管理授权，使产品隔离同时存在于 API、领域服务、数据库约束和 React 管理面。

## 主 PRD 上下文

- 目标：G-1、G-2
- 成功标准：SC-1、SC-2
- 需求：FR-1～FR-8，NFR-1～NFR-6

## 发现门禁

- [x] 盘点所有用户、设备、激活码、绑定、Profile、租约、用量、审计、幂等、管理查询的生产者和消费者。
- [x] 与商业化 Phase 2 锁定同一组 RBAC 迁移/API 写入集：商业化阶段拥有权限目录/角色生命周期/React 通用壳，本阶段先交付 product 范围约束和跨产品拒绝；本阶段未新建第二套角色表。
- [x] 检查最新迁移编号、历史数据量、空值比例和跨表引用，采用迁移 0023 的先扩展/回填/兼容方案；真实数据量与 PostgreSQL 回放仍待环境验证。
- [ ] 明确旧 autoLive 客户端兼容窗口、遥测、下线条件和默认映射移除时间。

## 范围

### 包含

- `products`、`user_products`、产品绑定会话和产品范围用户角色关系；固定权限目录、角色 CRUD 和 React 通用壳由商业化 Phase 2 同批交付。
- 设备、激活码、绑定、租约、用量、审计的产品字段、索引、唯一约束、外键/领域引用和幂等 scope。
- 管理 API/React 产品授权、筛选和 403 状态。
- 历史数据回填、兼容发布和收紧门禁。

### 不包含

- 不实现订阅/订单/席位，不引入 ABAC 或数据库拆分。
- 不允许客户端在已登录会话中切换产品。

## 实施清单

- [x] OpenAPI 增加稳定 ProductCode、产品会话上下文、管理筛选和跨产品错误码；生成类型由契约生成器同步。
- [x] 新增不可变 `products` 并种子化 `autolive`/`douyin_desktop`；新增全局用户到产品成员关系。
- [x] 为产品业务表先加可空字段、回填 `autolive`，验证引用；非空与终态产品范围约束留待后续收紧迁移。
- [x] 兼容阶段的首个迁移只做扩展：保留旧主键/唯一键/外键与旧 SQL 写路径，通过默认 `autolive` 和默认成员关系承接旧代码；真正的复合键和 `NOT NULL` 收紧另立后续迁移。
- [ ] 将设备唯一性改为 `(product, device_id)`；迁移 0023 仍保留旧全局设备键，避免兼容发布破坏旧写路径。
- [x] 登录/激活确定产品并写入会话；后续提示与会话不一致时 fail-closed。
- [ ] 固定权限目录、自定义角色生命周期和产品角色分配仍由商业化 Phase 2 交付；本阶段使用现有 `admin/user` 兼容模型，并将内建本地管理员视为跨产品查询兼容边界。
- [ ] React 管理面完整的授权产品上下文、导航/路由/操作权限壳仍待商业化 Phase 2；本阶段已完成服务端列表筛选、403 和 OpenAPI 类型同步。
- [ ] 为 `super_admin` 跨产品写操作增加确认、理由、幂等和审计；本阶段只保留内建管理员的低风险列表兼容范围。
- [ ] 为兼容默认建立计数/告警，达到下线条件后将新客户端 product 收紧为必填并删除永久默认路径。
- [x] 同步商业化 PRD 依赖、产品/系统/管理/数据库文档，并明确本阶段已实现范围和剩余 P0 阶段。

## 验证清单

- [ ] 同一 `device_id` 在两个产品独立存在；同产品重复和跨产品引用按契约拒绝。当前旧全局设备键仍在，需后续迁移收紧。
- [x] 在 Task 3–5 发布前，legacy SQL 省略 `product` 的写入保留默认 `autolive` 和默认成员关系的兼容路径；真实 PostgreSQL 回放待 `TEST_POSTGRES_URL`。
- [x] 跨产品激活码、会话、Profile、租约、用量、审计详情与写操作已由 Go 单元/HTTP/SQLMock 覆盖；真实 PostgreSQL 集成仍待运行。
- [x] 普通管理员省略/伪造 product 只能看到会话所属产品；内建本地管理员可省略查询参数查看全部或显式收窄，行为已由 HTTP 集成测试覆盖。
- [ ] 历史数据全部回填 `autolive`、无孤立引用的真实数据验证待 PostgreSQL 环境；迁移静态契约和集成测试已准备。
- [ ] PostgreSQL integration、React 权限/筛选和迁移回放未全部通过；完整 `go test -race ./...` 已通过，桌面端 API 检查缺少本地生成器但已用管理端同版本生成器完成无漂移比对。

## 退出标准

- [x] 已完成的服务端产品隔离不依赖客户端诚实或前端筛选；列表查询由 HTTP、服务层和仓储共同收窄。
- [x] 后续 Profile、模型和商业阶段可消费本阶段的 `ProductCode`、会话和成员关系事实；6h/24h 授权、委托凭证和商业权限仍未实现。

## 本轮实现与验证矩阵

| 场景 | 证据 | 状态与边界 |
| --- | --- | --- |
| 登录→激活→心跳→Profile 的产品绑定 | `backend/internal/httpapi/auth_test.go` 的产品登录、激活/心跳不匹配和 Profile 测试 | `autolive` 完整路径与 `douyin_desktop` 会话/跨产品拒绝已覆盖；两产品均完成真实 PostgreSQL 链路仍待 E2E |
| 同一设备标识的产品边界 | `backend/internal/service/product_isolation_task4_test.go`、`auth_test.go` | 服务层资源归属和会话产品拒绝已覆盖；数据库最终 `(product, device_id)` 唯一键尚未切换 |
| 管理列表省略/显式/非法/重复/跨产品 product | `backend/internal/httpapi/product_scope_integration_test.go`、`product_scope_test.go` | Memory HTTP 实际 items/total、400、403 已覆盖，包含用户列表、用户设备子列表和七类分页列表；全局用户授权/管理写操作由内建管理员守卫覆盖 |
| 快照源与归一化源产品分页 | `backend/internal/service/product_snapshot_page_test.go`、`backend/internal/store/postgres_repository_test.go` | 快照回退不调用 normalized 读者；SQLMock 检查 count/list 均带 product 谓词 |
| 迁移重放、旧写入默认值、重启 | `backend/migrations/postgres_integration_test.go`、`backend/internal/store/postgres_integration_test.go` | 需要 `TEST_POSTGRES_URL`；本轮未执行真实数据库，不能标记通过 |

本轮实际命令记录：`cd backend && go test ./...`、`go vet ./...`、`go build ./...`、`go test -race ./...`、`go test -race ./internal/service ./internal/store`、专项产品测试、`bash tools/check-document-references.sh`、`git diff --check`、管理端 `pnpm api:check` 均通过。桌面端 `pnpm api:check` 因 `openapi-typescript` 未安装而未通过，但已用管理端同版本生成器对两份生成类型做无落盘比对并通过；`TEST_POSTGRES_URL` 未设置，真实 PostgreSQL 未验证。

## 发现/决策

- 2026-08-22：用户确认单库硬逻辑隔离、全局用户身份和产品成员关系。
