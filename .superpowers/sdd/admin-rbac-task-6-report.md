# Task 6：React 权限壳和 RBAC 管理页报告

日期：2026-08-22
分支：`dev-2.0`
范围：仅 `admin-web` 与本任务报告；未修改 `desktop/`、Rust/Tauri、桌面端契约或桌面端测试。

## 改动文件

- `admin-web/package.json`
- `admin-web/pnpm-lock.yaml`
- `admin-web/src/app/router.tsx`
- `admin-web/src/components/AppLayout.tsx`
- `admin-web/src/features/activation-codes/ActivationCodesPage.tsx`
- `admin-web/src/features/admin-rbac/AdminForbiddenPage.tsx`
- `admin-web/src/features/admin-rbac/AdminRbacPage.tsx`
- `admin-web/src/features/admin-rbac/adminRbac.test.mjs`
- `admin-web/src/features/admin-rbac/adminRbacModel.ts`
- `admin-web/src/features/admin-rbac/adminRouteMeta.ts`
- `admin-web/src/features/admin-rbac/useAdminAuthorization.ts`
- `admin-web/src/features/audit-logs/AuditLogsPage.tsx`
- `admin-web/src/features/devices/DeviceManagementPage.tsx`
- `admin-web/src/features/model-leases/ModelLeasesPage.tsx`
- `admin-web/src/features/model-pool/ModelPoolPage.tsx`
- `admin-web/src/features/security/AdminSecurityPage.tsx`
- `admin-web/src/features/users/UserManagementPage.tsx`
- `admin-web/src/types/api.ts`

## 主要实现

- 新增 `useAdminAuthorization()`，固定使用 React Query key `['admin-me']` 读取 `/api/v1/admin/me`，缺失数据、加载中、请求失败均 fail-closed。
- 路由和菜单改为复用同一份权限元数据，直接 URL 未授权显示 `AdminForbiddenPage`，授权信息加载失败显示 Ant Design 加载/错误态。
- 新增 RBAC 管理页，覆盖角色列表、权限分组、角色创建/编辑/删除、产品范围用户角色替换，以及内建超管只读禁用控件。
- 根据审查修复权限 Tree：角色表单使用 Ant Design Form 的 `valuePropName="checkedKeys"`、`trigger="onCheck"` 和 `getValueFromEvent`，统一归一化数组及 `{ checked, halfChecked }` 事件，确保创建/编辑提交读取勾选权限。
- 根据审查修复产品范围状态：产品授权完成前查询保持禁用；非全局管理员强制遵循 `authorization.product`，全局超管保留自己的产品选择；删除未使用的 `assignableRoles`。
- 既有管理页接入固定读写权限控制，读权限保留列表/详情，写权限控制创建、编辑、禁用、换密钥、测试、回收等操作按钮；服务端 403 显示明确错误状态。
- 前端测试脚本改为 `tsx --test`，以支持新增 TS/TSX 测试入口。

## 执行命令与结果

- `cd admin-web && pnpm api:check`：通过
- `cd admin-web && pnpm typecheck`：通过
- `cd admin-web && pnpm test`：通过（18/18）
- `cd admin-web && pnpm build`：通过
- `git diff --check`：通过

## 测试覆盖点

- 权限集合缺失、加载中、错误均不放行
- 菜单过滤与直接 URL 守卫复用同一元数据
- 403 无权限 fixture
- 权限变更后重新取数
- 角色表单校验与提交重试辅助
- 权限 Tree 的数组 `checkedKeys` 与 `{ checked, halfChecked }` 事件均只回写权限叶子节点
- 产品授权加载/错误 fail-closed、非全局管理员产品强制跟随授权、全局超管保留选择

## 未验证项

- 未运行桌面端相关验证；按任务要求明确跳过
- 未做真实浏览器手工点击冒烟，仅完成类型检查、Node 测试和生产构建

## 备注

- `vite build` 产生现有大包体积告警（chunk > 500 kB），不阻断当前任务验收；本次未扩展到额外的拆包优化任务
