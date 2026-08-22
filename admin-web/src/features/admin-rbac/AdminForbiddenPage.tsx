import { Button, Result } from 'antd';
import { useNavigate } from 'react-router-dom';
import { FORBIDDEN_RESULT_TEXT } from './adminRbacModel';

export function AdminForbiddenPage({
  title = FORBIDDEN_RESULT_TEXT,
  description = '当前会话没有访问该页面所需的权限，请联系超级管理员调整角色后重试。',
}: {
  title?: string;
  description?: string;
}) {
  const navigate = useNavigate();

  return (
    <Result
      status="403"
      title={title}
      subTitle={description}
      extra={
        <Button type="primary" onClick={() => navigate('/', { replace: true })}>
          返回系统概览
        </Button>
      }
    />
  );
}
