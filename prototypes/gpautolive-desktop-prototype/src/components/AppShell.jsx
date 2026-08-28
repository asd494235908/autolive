import {
  BorderOutlined,
  CloseOutlined,
  MinusOutlined,
} from "@ant-design/icons";
import { Badge, Button, Tooltip } from "antd";

export function AppShell({ onWindowAction, children }) {
  return (
    <div className="app-shell">
      <header className="app-titlebar">
        <div className="app-brand">
          <img src="/app-icon.png" alt="GpAutoLive" />
          <strong>GpAutoLive</strong>
          <span>本地有序播放池引擎</span>
        </div>
        <div className="titlebar-status" aria-label="会话状态">
          <Badge status="success" text="设备已授权" />
          <span className="account-name">kangyun</span>
          <Button size="small" type="text" onClick={() => onWindowAction("退出登录")}>退出登录</Button>
        </div>
        <div className="window-actions" aria-label="窗口控制">
          <Tooltip title="最小化（原型演示）">
            <Button aria-label="最小化窗口" type="text" icon={<MinusOutlined />} onClick={() => onWindowAction("最小化")} />
          </Tooltip>
          <Tooltip title="最大化或还原（原型演示）">
            <Button aria-label="最大化或还原窗口" type="text" icon={<BorderOutlined />} onClick={() => onWindowAction("最大化或还原")} />
          </Tooltip>
          <Tooltip title="关闭（原型演示）">
            <Button className="window-close" aria-label="关闭窗口" type="text" icon={<CloseOutlined />} onClick={() => onWindowAction("关闭")} />
          </Tooltip>
        </div>
      </header>

      <nav className="app-nav" aria-label="桌面端页面导航">
        <span className="nav-label">工作区</span>
        <Button size="small" type="primary" aria-current="page">主页</Button>
        <div className="nav-spacer" />
        <span className="license-copy">授权剩余 364 天 · 设备 1/3</span>
      </nav>

      <main className="app-content">{children}</main>
    </div>
  );
}
