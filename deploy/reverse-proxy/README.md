# AutoLive HTTPS 入口

此目录提供生产边缘入口模板。API 容器继续只监听共享网络内的 `:8080`，管理端容器的 HTTP 端口只绑定宿主机回环地址 `127.0.0.1:18090`；公网流量必须由宿主机 Nginx 在 443 端口终止 TLS。Go 服务保留内部 HTTP hop，不代表公网允许 HTTP。

## 发布前配置

1. 将 `deploy/.env.example` 复制到受控部署目录，替换所有占位值，并保持：
   - `APP_DEPLOYMENT_ENV=production`
   - `APP_PUBLIC_BASE_URL=https://<实际域名>`
   - `APP_ALLOW_INSECURE_HTTP=false`
   - `AUTOLIVE_ADMIN_HTTP_BIND=127.0.0.1`
2. 将 `nginx.conf` 复制到宿主机 Nginx 配置目录，替换 `admin.example.com` 和证书路径；证书和私钥不得进入仓库或镜像。
3. 先执行 `nginx -t`，再加载配置。HTTP 入口只允许 308 跳转到同域 HTTPS，不得直接代理应用。
4. 桌面端正式构建使用 `VITE_CONTROL_PLANE_BASE_URL=https://<实际域名>`；Tauri capability 必须保留明确的 HTTPS 域名，不得改成任意 URL 通配。
5. 使用 `node tools/check-https-publish.mjs --env-file deploy/.env.example --reverse-proxy deploy/reverse-proxy/nginx.conf --capability desktop/src-tauri/capabilities/default.json --admin-nginx admin-web/nginx.conf`（或实际受控 env 文件）执行发布门禁。该检查不读取或打印密码、Token、密钥内容。

## 开发模式

内存模式可不设置 `APP_PUBLIC_BASE_URL`，并可由 Vite/浏览器使用本地 HTTP。若 staging 仍使用 HTTP，必须显式设置 `APP_ALLOW_INSECURE_HTTP=true`；生产环境即使设置该值也会被 Go 配置校验拒绝。
