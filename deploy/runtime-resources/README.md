# Autolive 运行资源静态发布

本目录只描述 `v0.1.0` 已预构建运行资源的安装、受限发布和验收。资源服务器不接收源码，也不执行 Node、Rust、Python 项目构建；Caddy 二进制和资源发布树必须在本地开发机或 CI 预先构建/准备。

## 固定路径与身份

- Caddy 监听 `0.0.0.0:7088`，静态根为 `/fs/autolive-resources`。
- 正式目录为 `/fs/autolive-resources/autolive-resources/v0.1.0/<scope>/`。
- 上传 staging 为 `/fs/autolive-resources-staging/<upload-id>/v0.1.0/<scope>/`，发布成功后才会原子移动到正式目录。
- Caddy 使用无登录的 `autolive-resources:autolive-resources` 用户/组，只读正式目录，日志写入 `/var/log/autolive-resources`。
- 发布密钥使用无登录的 `autolive-deploy` 身份，只允许 forced command dispatcher；该身份只写 staging 和已授权的发布路径，不允许普通 shell。
- `authorized_keys` 应使用 `command="/usr/local/sbin/autolive-resource-deploy-dispatcher",restrict`，并单独保存部署私钥和服务器 host key。私钥、密码、Token 不得写入仓库、日志或聊天记录。

## 本地安装顺序

1. 在本地下载并校验官方预构建 Caddy 二进制，安装为 `/usr/local/lib/autolive-resources/caddy`，不要在服务器编译 Caddy。
2. 安装本目录的 `Caddyfile`、`autolive-resources.service`、`autolive-resource-deploy-dispatcher` 和 `publish-runtime-resources` 到对应的 `/etc`、`/usr/local/sbin` 路径，并设置 root 可写、部署身份可执行的权限。
3. 从 rsync 官方源码包提取 `support/rrsync`，不要使用 Debian 自带的 `/usr/bin/rrsync`：

   `https://download.samba.org/pub/rsync/src/rsync-3.4.4.tar.gz`

   发布包 SHA-256：`bd88cf82fa653da32314fb229136407c5c90f80d1758d8f4b091767877d8fa96`

   提取后的 `support/rrsync` SHA-256：`7bc4950a886bc2f4986b8a85fe492b8b3612a0f7edab031ab79c66fca0390970`

   将脚本安装为 `/usr/local/lib/autolive-resources/rrsync`，启用密钥前再次校验 SHA-256，并运行：

   ```sh
   /usr/local/lib/autolive-resources/rrsync -help /fs/autolive-resources-staging
   ```

   自检输出必须包含 `-wo`、`-no-overwrite` 和 `-munge`。该脚本继续调用系统 `/usr/bin/rsync`；服务端不构建 rsync。

4. 创建固定目录和权限：

   ```sh
   install -d -o autolive-deploy -g autolive-resources -m 2770 /fs/autolive-resources
   install -d -o autolive-deploy -g autolive-deploy -m 0750 /fs/autolive-resources-staging
   install -d -o autolive-resources -g autolive-resources -m 0750 /var/log/autolive-resources
   install -d -o autolive-deploy -g autolive-deploy -m 0750 /var/lock/autolive-resources
   ```

   正式目录的写入权限需要按实际发布用户与 Caddy 用户的组策略最小化配置；Caddy 只需读，发布器只需创建并移动经过校验的 release 目录。

5. 启动服务并检查：

   ```sh
   sudo systemctl daemon-reload
   sudo systemctl enable --now autolive-resources
   sudo systemctl is-active autolive-resources
   ss -lnt | grep ':7088'
   ```

   发布器依赖 Debian 12 的 `/usr/bin/python3` 标准库和 `/usr/bin/flock`（util-linux）；这两个工具只负责校验 inventory 和 release 锁，不执行项目构建。

## 部署顺序

CI 对每个 scope 使用唯一的 `upload-id`（格式为正整数 run id 和正整数 attempt，以短横线连接），通过受限 SSH key 将已生成的 `autolive-deploy-inventory.json` 和资源文件上传到 run-scoped staging，然后调用：

```sh
publish-runtime-resources v0.1.0 <scope> <upload-id>
```

publisher 按 release 加锁，先原子认领 staging，再拒绝 symlink、隐藏项、额外文件、缺失文件、错误大小和错误 SHA-256。正式目录不存在时才原子发布；已存在目录只有在与 inventory 完全一致时才作为幂等重复发布成功，任何差异都失败。不要手工合并旧 run staging，也不要用任意路径参数调用脚本。

## 公网验收

默认生产地址固定为 `http://101.96.208.132:7088/autolive-resources/v0.1.0/`。本地执行：

```sh
node deploy/runtime-resources/verify-server.mjs desktop/src-tauri/runtime-resources.json
```

验证器使用 Node `fetch` 检查 HEAD 的 `Content-Length`、完整 GET 和 SHA-256、`bytes=0-15` 的 `206`/`Content-Range`/16 字节，以及越界 Range 的 `416`；重定向会失败。可选第二个参数只用于本地 fixture 或明确的验收地址，不改变生产客户端固定地址。

服务器重启后再次运行同一命令。若公网地址不可达，记录本地 `systemctl is-active` 和监听证据，并由网络管理员处理公网到内网 `7088` 的路由/防火墙映射；不要为了验收连接或修改服务器。
