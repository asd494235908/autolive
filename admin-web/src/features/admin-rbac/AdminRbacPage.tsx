import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  Alert,
  App,
  Button,
  Card,
  Checkbox,
  Descriptions,
  Empty,
  Form,
  Input,
  Modal,
  Result,
  Select,
  Space,
  Table,
  Tag,
  Tree,
  Typography,
} from 'antd';
import { useEffect, useMemo, useRef, useState } from 'react';
import { apiClient, ApiClientError, createRequestId } from '../../api/client';
import type {
  AdminPermissionCode,
  AdminPermissionsResponse,
  AdminRole,
  AdminRoleCreateRequest,
  AdminRoleEnvelope,
  AdminRoleListResponse,
  AdminRoleUpdateRequest,
  ProductCode,
  ReplaceUserAdminRolesRequest,
  UserAdminRolesResponse,
} from '../../types/api';
import {
  buildPermissionTreeData,
  createRetryableSubmission,
  getDomainLabel,
  groupPermissionsByDomain,
  validateRoleDraft,
} from './adminRbacModel';
import { useAdminAuthorization } from './useAdminAuthorization';

type RoleFormValues = {
  code: string;
  name: string;
  product: ProductCode;
  permissions: AdminPermissionCode[];
};

type AssignmentFormValues = {
  role_codes: string[];
};

const productOptions: Array<{ label: string; value: ProductCode }> = [
  { label: 'AutoLive', value: 'autolive' },
  { label: '抖音桌面端', value: 'douyin_desktop' },
];

function isForbidden(error: unknown) {
  return error instanceof ApiClientError && error.status === 403;
}

function formatProduct(product: ProductCode | null) {
  if (product === null) {
    return '全局';
  }
  return product === 'autolive' ? 'AutoLive' : '抖音桌面端';
}

export function AdminRbacPage() {
  const authorization = useAdminAuthorization();
  const { message, modal } = App.useApp();
  const queryClient = useQueryClient();
  const [roleForm] = Form.useForm<RoleFormValues>();
  const [assignmentForm] = Form.useForm<AssignmentFormValues>();
  const [selectedProduct, setSelectedProduct] = useState<ProductCode>('autolive');
  const [roleModalMode, setRoleModalMode] = useState<'create' | 'edit' | null>(null);
  const [editingRole, setEditingRole] = useState<AdminRole | null>(null);
  const [detailRole, setDetailRole] = useState<AdminRole | null>(null);
  const [roleSubmitError, setRoleSubmitError] = useState<string | null>(null);
  const [assignmentSubmitError, setAssignmentSubmitError] = useState<string | null>(null);
  const [assignmentUserInput, setAssignmentUserInput] = useState('');
  const [assignmentUserId, setAssignmentUserId] = useState('');
  const [assignmentProduct, setAssignmentProduct] = useState<ProductCode>('autolive');
  const [assignmentModalOpen, setAssignmentModalOpen] = useState(false);
  const roleSubmission = useRef(createRetryableSubmission(createRequestId));
  const assignmentSubmission = useRef(createRetryableSubmission(createRequestId));

  useEffect(() => {
    if (authorization.product) {
      setSelectedProduct((current) => current ?? authorization.product);
      setAssignmentProduct((current) => current ?? authorization.product);
    }
  }, [authorization.product]);

  const permissionsQuery = useQuery({
    queryKey: ['admin-permissions'],
    queryFn: () => apiClient.get<AdminPermissionsResponse>('/api/v1/admin/permissions'),
    enabled: authorization.can('roles.read'),
  });

  const rolesQuery = useQuery({
    queryKey: ['admin-roles', selectedProduct],
    queryFn: () =>
      apiClient.get<AdminRoleListResponse>('/api/v1/admin/roles', {
        query: { product: selectedProduct },
      }),
    enabled: authorization.can('roles.read'),
  });

  const assignmentsQuery = useQuery({
    queryKey: ['admin-user-roles', assignmentUserId, assignmentProduct],
    queryFn: () =>
      apiClient.get<UserAdminRolesResponse>(`/api/v1/admin/users/${assignmentUserId}/roles`, {
        query: { product: assignmentProduct },
      }),
    enabled: authorization.can('roles.assign') && assignmentUserId.length > 0,
  });

  const createRoleMutation = useMutation({
    mutationFn: (values: AdminRoleCreateRequest) =>
      apiClient.post<AdminRoleEnvelope>('/api/v1/admin/roles', {
        body: values,
        headers: { 'Idempotency-Key': roleSubmission.current.current() },
      }),
    onSuccess: async (response) => {
      roleSubmission.current.reset();
      setRoleSubmitError(null);
      setRoleModalMode(null);
      setEditingRole(null);
      roleForm.resetFields();
      void message.success(`角色已创建：${response.role.name}（request_id：${response.request_id}）`);
      await queryClient.invalidateQueries({ queryKey: ['admin-roles'] });
    },
    onError: (error) => {
      setRoleSubmitError(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}；再次提交会复用当前幂等键。`
          : '创建角色失败；再次提交会复用当前幂等键。'
      );
    },
  });

  const updateRoleMutation = useMutation({
    mutationFn: ({ roleCode, values }: { roleCode: string; values: AdminRoleUpdateRequest }) =>
      apiClient.request<AdminRoleEnvelope>(`/api/v1/admin/roles/${roleCode}`, {
        method: 'PATCH',
        body: values,
        headers: { 'Idempotency-Key': roleSubmission.current.current() },
      }),
    onSuccess: async (response) => {
      roleSubmission.current.reset();
      setRoleSubmitError(null);
      setRoleModalMode(null);
      setEditingRole(null);
      roleForm.resetFields();
      void message.success(`角色已更新：${response.role.name}（request_id：${response.request_id}）`);
      await queryClient.invalidateQueries({ queryKey: ['admin-roles'] });
    },
    onError: (error) => {
      setRoleSubmitError(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}；再次提交会复用当前幂等键。`
          : '更新角色失败；再次提交会复用当前幂等键。'
      );
    },
  });

  const deleteRoleMutation = useMutation({
    mutationFn: (roleCode: string) =>
      apiClient.request<void>(`/api/v1/admin/roles/${roleCode}`, {
        method: 'DELETE',
        headers: { 'Idempotency-Key': createRequestId() },
      }),
    onSuccess: async () => {
      void message.success('角色已删除');
      await queryClient.invalidateQueries({ queryKey: ['admin-roles'] });
    },
    onError: (error) => {
      void message.error(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}`
          : '删除角色失败'
      );
    },
  });

  const replaceAssignmentsMutation = useMutation({
    mutationFn: (values: ReplaceUserAdminRolesRequest) =>
      apiClient.request<UserAdminRolesResponse>(`/api/v1/admin/users/${assignmentUserId}/roles`, {
        method: 'PUT',
        body: values,
        headers: { 'Idempotency-Key': assignmentSubmission.current.current() },
      }),
    onSuccess: async (response) => {
      assignmentSubmission.current.reset();
      setAssignmentSubmitError(null);
      setAssignmentModalOpen(false);
      void message.success(`用户角色已替换（request_id：${response.request_id}）`);
      await queryClient.invalidateQueries({ queryKey: ['admin-user-roles', assignmentUserId, assignmentProduct] });
    },
    onError: (error) => {
      setAssignmentSubmitError(
        error instanceof ApiClientError
          ? `${error.message}${error.requestId ? `（request_id：${error.requestId}）` : ''}；再次提交会复用当前幂等键。`
          : '替换用户角色失败；再次提交会复用当前幂等键。'
      );
    },
  });

  useEffect(() => {
    const roleCodes = assignmentsQuery.data?.assignments.map((item) => item.role_code) ?? [];
    assignmentForm.setFieldsValue({ role_codes: roleCodes });
  }, [assignmentForm, assignmentsQuery.data]);

  const permissionTreeData = useMemo(
    () => buildPermissionTreeData(permissionsQuery.data?.permissions ?? []),
    [permissionsQuery.data]
  );

  const groupedPermissions = useMemo(
    () => groupPermissionsByDomain(permissionsQuery.data?.permissions ?? []),
    [permissionsQuery.data]
  );

  const assignableRoles = useMemo(
    () =>
      (rolesQuery.data?.roles ?? []).filter((role) => role.product === selectedProduct || role.product === null),
    [rolesQuery.data, selectedProduct]
  );

  const assignmentOptions = useMemo(
    () =>
      (rolesQuery.data?.roles ?? [])
        .filter((role) => role.product === assignmentProduct || role.product === null)
        .map((role) => ({
          label: `${role.name}（${role.code}）`,
          value: role.code,
          disabled: role.built_in,
        })),
    [assignmentProduct, rolesQuery.data]
  );

  const roleColumns = [
    { title: '角色名称', dataIndex: 'name', key: 'name' },
    { title: '角色代码', dataIndex: 'code', key: 'code' },
    {
      title: '产品范围',
      key: 'product',
      render: (_: unknown, role: AdminRole) => formatProduct(role.product),
    },
    {
      title: '内建',
      key: 'built_in',
      render: (_: unknown, role: AdminRole) => (role.built_in ? <Tag color="gold">内建</Tag> : <Tag>自定义</Tag>),
    },
    {
      title: '权限数',
      key: 'permission_count',
      render: (_: unknown, role: AdminRole) => role.permissions.length,
    },
    {
      title: '操作',
      key: 'actions',
      render: (_: unknown, role: AdminRole) => (
        <Space>
          <Button type="link" onClick={() => setDetailRole(role)}>
            详情
          </Button>
          <Button
            disabled={!authorization.can('roles.manage') || role.built_in}
            onClick={() => {
              roleSubmission.current.reset();
              setRoleSubmitError(null);
              setEditingRole(role);
              setRoleModalMode('edit');
              roleForm.setFieldsValue({
                code: role.code,
                name: role.name,
                product: role.product ?? selectedProduct,
                permissions: role.permissions,
              });
            }}
          >
            编辑
          </Button>
          <Button
            danger
            disabled={!authorization.can('roles.manage') || role.built_in}
            loading={deleteRoleMutation.isPending}
            onClick={() => {
              modal.confirm({
                title: '确认删除角色',
                content: `角色“${role.name}”删除后不可恢复；若仍有用户绑定，服务端会拒绝。`,
                okText: '确认删除',
                cancelText: '取消',
                okButtonProps: { danger: true },
                onOk: () => deleteRoleMutation.mutateAsync(role.code),
              });
            }}
          >
            删除
          </Button>
        </Space>
      ),
    },
  ];

  const submitRoleForm = async () => {
    const values = await roleForm.validateFields();
    const validationErrors = validateRoleDraft(values);
    if (validationErrors.length > 0) {
      roleForm.setFields(
        validationErrors.map(([field, message]) => ({
          name: [field] as ['code' | 'name' | 'product' | 'permissions'],
          errors: [message],
        }))
      );
      return;
    }

    const payload = {
      code: values.code.trim(),
      name: values.name.trim(),
      product: values.product,
      permissions: [...values.permissions].sort(),
    };

    if (roleModalMode === 'edit' && editingRole) {
      await updateRoleMutation.mutateAsync({
        roleCode: editingRole.code,
        values: payload,
      });
      return;
    }

    await createRoleMutation.mutateAsync(payload);
  };

  const submitAssignments = async () => {
    const values = await assignmentForm.validateFields();
    await replaceAssignmentsMutation.mutateAsync({
      product: assignmentProduct,
      role_codes: [...(values.role_codes ?? [])].sort(),
    });
  };

  if (isForbidden(permissionsQuery.error) || isForbidden(rolesQuery.error)) {
    return (
      <Result
        status="403"
        title="无权读取 RBAC 配置"
        subTitle="服务端拒绝了当前角色的 RBAC 读取请求，请刷新权限后重试。"
        extra={
          <Button
            type="primary"
            onClick={() => {
              void authorization.refresh();
              void permissionsQuery.refetch();
              void rolesQuery.refetch();
            }}
          >
            重新验证权限
          </Button>
        }
      />
    );
  }

  return (
    <Space direction="vertical" size="large" style={{ width: '100%' }}>
      <Space align="start" style={{ width: '100%', justifyContent: 'space-between' }}>
        <div>
          <Typography.Title level={2} style={{ margin: 0 }}>
            RBAC 管理
          </Typography.Title>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
            读取固定权限目录、角色列表，并按产品范围替换用户角色；内建超级管理员角色只读显示，仍由服务端做最终拒绝。
          </Typography.Paragraph>
        </div>

        <Space>
          <Select<ProductCode>
            value={selectedProduct}
            style={{ width: 160 }}
            options={productOptions}
            disabled={!authorization.isSuperAdmin && !!authorization.product}
            onChange={(value) => setSelectedProduct(value)}
          />
          <Button
            type="primary"
            disabled={!authorization.can('roles.manage')}
            onClick={() => {
              roleSubmission.current.reset();
              setRoleSubmitError(null);
              setEditingRole(null);
              setRoleModalMode('create');
              roleForm.setFieldsValue({
                code: '',
                name: '',
                product: selectedProduct,
                permissions: [],
              });
            }}
          >
            创建角色
          </Button>
        </Space>
      </Space>

      {permissionsQuery.isError && !isForbidden(permissionsQuery.error) ? (
        <Alert
          type="error"
          showIcon
          message="权限目录加载失败"
          description={
            <Space direction="vertical" size="small">
              <Typography.Text>
                {permissionsQuery.error instanceof ApiClientError
                  ? `${permissionsQuery.error.message}${permissionsQuery.error.requestId ? `（request_id：${permissionsQuery.error.requestId}）` : ''}`
                  : '发生未知错误'}
              </Typography.Text>
              <Button onClick={() => void permissionsQuery.refetch()}>重试</Button>
            </Space>
          }
        />
      ) : null}

      {rolesQuery.isError && !isForbidden(rolesQuery.error) ? (
        <Alert
          type="error"
          showIcon
          message="角色列表加载失败"
          description={
            <Space direction="vertical" size="small">
              <Typography.Text>
                {rolesQuery.error instanceof ApiClientError
                  ? `${rolesQuery.error.message}${rolesQuery.error.requestId ? `（request_id：${rolesQuery.error.requestId}）` : ''}`
                  : '发生未知错误'}
              </Typography.Text>
              <Button onClick={() => void rolesQuery.refetch()}>重试</Button>
            </Space>
          }
        />
      ) : null}

      <Card title="角色列表">
        <Table<AdminRole>
          rowKey="code"
          columns={roleColumns}
          dataSource={rolesQuery.data?.roles ?? []}
          loading={rolesQuery.isLoading}
          pagination={false}
          locale={{
            emptyText: rolesQuery.isLoading ? '加载中...' : <Empty description="当前产品暂无角色" />,
          }}
        />
      </Card>

      <Card title="权限分组">
        {permissionsQuery.isLoading ? (
          <Typography.Text type="secondary">加载中…</Typography.Text>
        ) : (
          <Tree
            checkable
            selectable={false}
            checkedKeys={permissionsQuery.data?.permissions ?? []}
            treeData={permissionTreeData}
          />
        )}
      </Card>

      <Card
        title="产品范围用户角色替换"
        extra={
          <Button
            disabled={!authorization.can('roles.assign')}
            onClick={() => {
              const nextUserId = assignmentUserInput.trim();
              if (!nextUserId) {
                void message.warning('请先输入用户 ID');
                return;
              }
              setAssignmentUserId(nextUserId);
            }}
          >
            加载绑定
          </Button>
        }
      >
        <Space direction="vertical" size="middle" style={{ width: '100%' }}>
          <Space wrap>
            <Input
              aria-label="用户 ID"
              placeholder="输入用户 ID"
              value={assignmentUserInput}
              onChange={(event) => setAssignmentUserInput(event.target.value)}
              style={{ width: 220 }}
            />
            <Select<ProductCode>
              value={assignmentProduct}
              style={{ width: 160 }}
              options={productOptions}
              disabled={!authorization.isSuperAdmin && !!authorization.product}
              onChange={(value) => {
                assignmentSubmission.current.reset();
                setAssignmentSubmitError(null);
                setAssignmentProduct(value);
              }}
            />
            <Button
              disabled={!authorization.can('roles.assign')}
              onClick={() => {
                const nextUserId = assignmentUserInput.trim();
                if (!nextUserId) {
                  void message.warning('请先输入用户 ID');
                  return;
                }
                setAssignmentUserId(nextUserId);
              }}
            >
              查询当前绑定
            </Button>
          </Space>

          {assignmentUserId.length === 0 ? (
            <Empty description="输入用户 ID 后可查看并替换产品范围角色" />
          ) : assignmentsQuery.isLoading ? (
            <Typography.Text type="secondary">正在加载用户角色…</Typography.Text>
          ) : isForbidden(assignmentsQuery.error) ? (
            <Result
              status="403"
              title="无权读取用户角色"
              subTitle="当前会话没有 `roles.assign` 所需的委派能力。"
            />
          ) : assignmentsQuery.isError ? (
            <Alert
              type="error"
              showIcon
              message="用户角色读取失败"
              description={
                <Space direction="vertical" size="small">
                  <Typography.Text>
                    {assignmentsQuery.error instanceof ApiClientError
                      ? `${assignmentsQuery.error.message}${assignmentsQuery.error.requestId ? `（request_id：${assignmentsQuery.error.requestId}）` : ''}`
                      : '发生未知错误'}
                  </Typography.Text>
                  <Button onClick={() => void assignmentsQuery.refetch()}>重试</Button>
                </Space>
              }
            />
          ) : (
            <Space direction="vertical" size="middle" style={{ width: '100%' }}>
              <Descriptions bordered size="small" column={1}>
                <Descriptions.Item label="用户 ID">{assignmentUserId}</Descriptions.Item>
                <Descriptions.Item label="产品范围">{formatProduct(assignmentProduct)}</Descriptions.Item>
                <Descriptions.Item label="当前角色">
                  <Space wrap>
                    {(assignmentsQuery.data?.assignments ?? []).length === 0 ? (
                      <Typography.Text type="secondary">暂无角色绑定</Typography.Text>
                    ) : (
                      (assignmentsQuery.data?.assignments ?? []).map((assignment) => (
                        <Tag key={`${assignment.role_code}-${assignment.product}`}>
                          {assignment.role_code}
                        </Tag>
                      ))
                    )}
                  </Space>
                </Descriptions.Item>
              </Descriptions>
              <Button
                type="primary"
                disabled={!authorization.can('roles.assign')}
                onClick={() => {
                  assignmentSubmission.current.reset();
                  setAssignmentSubmitError(null);
                  assignmentForm.setFieldsValue({
                    role_codes: assignmentsQuery.data?.assignments.map((item) => item.role_code) ?? [],
                  });
                  setAssignmentModalOpen(true);
                }}
              >
                替换角色
              </Button>
            </Space>
          )}
        </Space>
      </Card>

      <Modal
        title={roleModalMode === 'edit' ? '编辑角色' : '创建角色'}
        open={roleModalMode !== null}
        confirmLoading={createRoleMutation.isPending || updateRoleMutation.isPending}
        destroyOnClose
        onCancel={() => {
          if (createRoleMutation.isPending || updateRoleMutation.isPending) {
            return;
          }
          roleSubmission.current.reset();
          setRoleSubmitError(null);
          setRoleModalMode(null);
          setEditingRole(null);
          roleForm.resetFields();
        }}
        onOk={() => {
          void submitRoleForm();
        }}
      >
        <Form<RoleFormValues>
          form={roleForm}
          layout="vertical"
          onValuesChange={() => {
            roleSubmission.current.reset();
            setRoleSubmitError(null);
          }}
        >
          <Form.Item label="角色代码" name="code" rules={[{ required: true, message: '请输入角色代码' }]}>
            <Input disabled={roleModalMode === 'edit'} />
          </Form.Item>
          <Form.Item label="角色名称" name="name" rules={[{ required: true, message: '请输入角色名称' }]}>
            <Input />
          </Form.Item>
          <Form.Item label="产品范围" name="product" rules={[{ required: true, message: '请选择产品范围' }]}>
            <Select options={productOptions} disabled={roleModalMode === 'edit' || (!authorization.isSuperAdmin && !!authorization.product)} />
          </Form.Item>
          <Form.Item
            label="权限集合"
            name="permissions"
            rules={[{ required: true, message: '至少选择一个权限' }]}
          >
            <Tree checkable selectable={false} treeData={permissionTreeData} />
          </Form.Item>
          {roleSubmitError ? <Alert type="error" showIcon message={roleSubmitError} /> : null}
        </Form>
      </Modal>

      <Modal
        title="替换用户角色"
        open={assignmentModalOpen}
        confirmLoading={replaceAssignmentsMutation.isPending}
        destroyOnClose
        onCancel={() => {
          if (replaceAssignmentsMutation.isPending) {
            return;
          }
          assignmentSubmission.current.reset();
          setAssignmentSubmitError(null);
          setAssignmentModalOpen(false);
        }}
        onOk={() => {
          void submitAssignments();
        }}
      >
        <Form<AssignmentFormValues>
          form={assignmentForm}
          layout="vertical"
          onValuesChange={() => {
            assignmentSubmission.current.reset();
            setAssignmentSubmitError(null);
          }}
        >
          <Form.Item label="角色列表" name="role_codes">
            <Checkbox.Group options={assignmentOptions} />
          </Form.Item>
          <Typography.Paragraph type="secondary" style={{ marginBottom: 0 }}>
            内建超级管理员角色保持禁用显示，服务端仍会拒绝越权委派。
          </Typography.Paragraph>
          {assignmentSubmitError ? <Alert type="error" showIcon message={assignmentSubmitError} /> : null}
        </Form>
      </Modal>

      <Modal
        title="角色详情"
        open={detailRole !== null}
        footer={null}
        onCancel={() => setDetailRole(null)}
      >
        {detailRole ? (
          <Space direction="vertical" size="middle" style={{ width: '100%' }}>
            <Descriptions bordered size="small" column={1}>
              <Descriptions.Item label="角色名称">{detailRole.name}</Descriptions.Item>
              <Descriptions.Item label="角色代码">{detailRole.code}</Descriptions.Item>
              <Descriptions.Item label="产品范围">{formatProduct(detailRole.product)}</Descriptions.Item>
              <Descriptions.Item label="类型">
                {detailRole.built_in ? <Tag color="gold">内建角色</Tag> : <Tag>自定义角色</Tag>}
              </Descriptions.Item>
            </Descriptions>
            {Object.entries(groupPermissionsByDomain(detailRole.permissions)).map(([domain, items]) => (
              <Card key={domain} size="small" title={getDomainLabel(domain)}>
                <Space wrap>
                  {items.map((permission) => (
                    <Tag key={permission}>{permission}</Tag>
                  ))}
                </Space>
              </Card>
            ))}
          </Space>
        ) : null}
      </Modal>
    </Space>
  );
}
