# Phase 4：管理查询 API

Parent PRD：[PRD：服务端商业化与生产就绪](../prd-server-commercial-production-readiness.md)
状态：Not Started
最后更新：2026-08-21

## 目标

提供订阅、订单、套餐版本和增强设备的管理列表/详情 API，所有筛选、排序和分页在 normalized PostgreSQL 中有界执行。

## 主 PRD 上下文

- 目标：G-1
- 成功标准：SC-1
- 需求：FR-1～FR-6，NFR-1、NFR-7
- 场景：场景 1

## 阶段发现门禁

- [ ] 重新检查 `backend/internal/httpapi/controlplane.go`、`backend/internal/service/*_options.go`、现有 page reader 模式
- [ ] 重新检查 `postgres_device_reader.go`、索引迁移 0017/0021 和查询计划测试
- [ ] 确认 Phase 3 的商业表/DTO 已稳定；若尚未完成则不得伪造订阅、订单或套餐版本查询
- [ ] 重新估算页大小、时间窗口和排序索引
- [ ] 主 PRD 假设仍成立，变更先同步文档

## 范围

### 包含

- subscriptions/orders/plan-versions 管理列表与详情。
- devices 用户、状态、在线、OS、客户端版本、心跳时间与排序筛选。
- 查询白名单、SQL 索引、OpenAPI 和服务端权限/审计语义。

### 不包含

- 不在本阶段创建支付或变更订阅。
- 不支持任意 SQL 字段/表达式排序或导出全库。

## 实施清单

- [ ] 先在 OpenAPI 定义四类资源的列表/详情路由、query enum、DTO 和错误响应。
- [ ] 分别创建领域 options、page reader 和脱敏 detail reader，不继续扩大一个通用 options 文件。
- [ ] 为 normalized PostgreSQL 实现 `COUNT + LIMIT/OFFSET` 和参数化白名单查询。
- [ ] 为 Memory 模式保留有界测试回退，生产 normalized 缺少 reader 时 fail-closed。
- [ ] 为组合筛选和排序补充复合索引，不为每个可能组合盲目建索引。
- [ ] 将路由加入固定低基数 metrics 模板和管理员权限测试。
- [ ] 更新 OpenAPI、错误码和 API 示例；不在本专项开发管理页面或消费端状态。

## 验证策略

使用 options 单元测试、SQLMock、真实 PostgreSQL 集成和 API 契约测试证明输入白名单、分页一致性、权限和查询计划。

## 验证清单

- [ ] 合法/非法筛选、空结果、页越界、时间反转、排序注入和无权限测试
- [ ] 组合筛选无 N+1，真实数据量 `EXPLAIN (ANALYZE, BUFFERS)` 符合容量目标
- [ ] `go test ./internal/service ./internal/store ./internal/httpapi`
- [ ] `go test -tags=postgres_integration ./internal/store`
- [ ] 现有消费端 OpenAPI 生成门禁只做兼容检查；不修改 React/Rust 业务代码

## 退出标准

- [ ] 四类查询资源的契约、SQL、索引、权限和界面筛选语义一致
- [ ] 不存在无界加载、任意排序或敏感字段泄露

## 阶段末多轮复核

- [ ] 1. 意图/覆盖；2. 正确性；3. 简化；4. 边界/命名；5. 重复/清理
- [ ] 6. 安全/隐私；7. 性能/容量；8. 验证充分性；9. 后续阶段；10. 主 PRD 同步

## 发现/决策

- 2026-08-21：阶段文件创建，本轮未开始实施。
