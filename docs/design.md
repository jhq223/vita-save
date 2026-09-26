# 实现结构

`src/app.rs` 串行调度后台任务。UI 线程只处理 Nivora 导航、输入、布局和绘制；磁盘、挂载、校验和网络任务在工作线程运行。进度保留最新状态，不积压逐文件消息。

| 模块 | 职责 |
| --- | --- |
| `saves` | app.db 只读快照、游戏列表、SFO 边界检查 |
| `backup` | 文件清单、流式复制、SHA-256、恢复日志 |
| `cloud` | WebDAV、`.vsave` 归档、10 秒 I/O 等待上限 |
| `platform` | EGL/GLES2、输入、PFS 挂载、Vita SQLite 内存 VFS |
| `ui` | 存档列表、详情、备份记录、云端、设置页面及任务对话框 |
| `native` | Rust `no_std` 内核及用户态挂载模块 |

## 本地备份

每个版本使用时间戳加 128 位随机值作为 ID。先写入独占的临时目录，逐文件同步；清单包含版本、Title ID、Save ID、目录和文件大小、SHA-256。完成后重命名到 `snapshots/`。

恢复顺序：校验源版本 → 按设置备份并校验当前存档 → 持久化恢复日志 → 写入文件 → 删除多余的数据文件 → 回读校验 → 删除日志。`backup_before_restore` 默认开启；开启时空间不足不会跳过自动备份。失败或取消后保留日志：有自动备份时回退到恢复前的存档，没有时重新写入日志指向的目标版本，并重新适配当前账户 ID。后续更改设置不改变已有日志的处理方式。

删除前检查所有恢复日志，拒绝删除日志引用的版本；将备份移入 `trash/` 后才删除文件，中途失败不会在备份列表中留下半个版本。

PFS 元数据不参与导出或覆盖。路径必须相对、无父目录跳转、无链接、无大小写冲突。下载时先验证清单，再创建文件；校验失败不发布本地版本。

## 平台

Nivora 和 PVR 绑定固定到 Git 提交；字体后端使用 ab_glyph，未启用 fontdb、cosmic-text 或 winit。EGL 上下文与输入由 Vita 宿主管理。

运行时分配 16 MiB SceLibc 堆供 PVR 使用、128 MiB newlib 堆和 2 MiB 主线程栈。release 打包关闭 `cargo-vita` 的符号裁剪，保留 `sceLibcHeapSize` 与 `sceUserMainThreadStackSize`，供 `vita-elf-create` 生成进程参数；构建脚本检查最终 ELF 中这些符号是否存在。

内核挂载模块从 VPK 安装路径 `ux0:app/VSAVE0001/module/vita-save-kernel.skprx` 加载；用户模块从 `app0:module/vita-save-user.suprx` 加载。两者在本次应用进程中仅加载一次；内核模块已驻留时接受 `0x8002D013`，不卸载该模块。挂载模块使用 SDK ABI，Rust 结构体有编译期大小检查。仅接受已知挂载 ID，按 SceAppMgr 模块 NID 选择固件偏移；未识别的固件不调用内部函数。用户指针经内核复制接口读取。挂载成功后，通过游戏原始目录读写解密后的文件；返回的挂载名称只交给 `sceAppMgrUmount`，不作为 `std::fs` 的访问路径。

SQLite 通过 `rusqlite` 的 `bundled` 功能引入，由 VitaSDK 编译内置 C 引擎并静态链接；`serialize` 用于载入只读数据库快照。Vita 编译禁用 Unix VFS、动态扩展和引擎内部线程支持，Rust 互斥锁覆盖连接创建、查询和销毁。SQLite 在 Vita 上仅使用内存 VFS，数据库文件由 Rust 读取。检测到未结束的 WAL/journal 时拒绝扫描；不会修改系统 app.db。TLS 使用 ureq、rustls、webpki-roots，以及固定到 `502a916e86dce91ffa9ad96ef62a6e9fc6f96755` 的 [vita-rust/ring](https://github.com/vita-rust/ring/tree/502a916e86dce91ffa9ad96ef62a6e9fc6f96755)。

系统键盘通过 `SceIme_stub` 调用 `sceImeOpen`、`sceImeUpdate`、`sceImeClose`，与 PVR 渲染并用。工作区、UTF-16 输入和回调状态在会话结束前保持地址稳定。Nivora 文本输入控件显示文本、光标及预编辑；密码只向 UI 提交掩码，禁用输入辅助。URL 使用基本拉丁键盘，用户名与密码保留 Unicode 输入。按确认后读取系统已提交缓冲区，预编辑回调只用于显示。键盘打开时应用不接收导航和触控；关闭后重置按键状态。

## 云端

上传先形成已校验归档，检查远端同名版本，再以 `If-None-Match: *` 直接 PUT 到版本文件。上传不依赖 MOVE。网络错误导致结果不明时不删除远端版本，避免误删另一设备刚上传的同名备份。服务器可能保留未完成文件，下载仍须通过完整归档校验。

连接测试依次读取目录、写入随机名称的小文件、读回比较及删除测试文件；失败信息标明请求阶段，清理失败时保留文件名。

下载只允许当前服务器、当前游戏集合中的直接子项，限制 XML、清单和载荷大小。下载及校验完成后才成为本地备份。下载不会直接修改游戏存档。
