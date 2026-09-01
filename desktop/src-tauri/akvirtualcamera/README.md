# AkVirtualCamera 运行资源目录

该目录是桌面端安装包的受信任资源根。正式构建前，发布流水线必须把通过
`desktop/tools/verify-akvirtualcamera-lock.mjs --require-release-ready` 的
`release-ready.json`、x86/x64 DirectShow 组件、x64 Assistant/Manager、x64 sidecar 和 C API 复制到这里；
签名证据留在 `desktop/third_party/akvirtualcamera/` 供发布校验，不进入运行时资源树。
对外分发所需的 `COPYING`、修改说明、对应源码清单和 SBOM 一并暂存；法务审核、GPU 基准
和兼容矩阵只留在发布证据目录，不进入运行时资源树。sidecar 固定位于
`bin/akvirtualcamera-sidecar-x64.exe`。

测试包故意只携带本说明，不会注册系统摄像头，也不会把未签名或不完整的组件
伪装成已安装。GPL 对应源码和构建输入仍以
`desktop/third_party/akvirtualcamera/` 为准。
