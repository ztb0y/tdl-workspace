# tdl-workspace

`tdl`、`tdl-web` 和 `tdl-web-ui` 的共享开发工作区。

本仓库是一个 Git superproject，不合并三个项目的 Git 历史。根仓库只负责固定一组可复现的源码版本；三个子项目仍然独立开发、测试和发布。

## 组成

- `tdl/`：官方 `iyear/tdl` 源码，默认固定在 `v0.20.3`。
- `tdl-web/`：Telegram 媒体浏览和下载扩展，只能通过 `tdl` 的扩展命令启动。
- `tdl-web-ui/`：独立的 React/Vite 前端项目。

## 获取源码

首次克隆：

```sh
git clone --recurse-submodules https://github.com/ztb0y/tdl-workspace.git
cd tdl-workspace
```

已有工作区初始化子模块：

```sh
git submodule update --init --recursive
```

检查当前固定版本：

```sh
git submodule status
```

## 源码联调

根目录的 `go.work` 会让 `tdl-web` 优先使用本地 `tdl`、`core` 和 `extension` 源码。这样可以在同一个工作区内调试 tdl 源码与扩展之间的调用关系。

源码模式构建：

```powershell
$env:GOWORK = "$(Get-Location)\go.work"
go test ./tdl-web/...
go build -o bin\tdl.exe ./tdl
go build -o bin\tdl-web.exe ./tdl-web
```

独立发布模式关闭 `go.work`：

```powershell
$env:GOWORK = "off"
go test ./...
go build ./...
```

尚未合并的 tdl 改动只应在 `tdl/` 的本地分支中测试。不要把未合并的子模块指针提交到 workspace 的 `main`。

## Windows 开发启动

需要：

- Go 1.25.8 或更高版本
- Node.js 22 或更高版本
- npm
- Rust 1.97.1

双击根目录的 `Start-Workspace.cmd`，或在终端执行：

```powershell
.\Start-Workspace.cmd
```

当前开发启动流程会：

1. 测试并构建 UI、官方 `tdl.exe`、`tdl-web.exe` 和 Slint launcher。
2. 在 `bin\client` 生成开发 runtime，不写入系统安装目录。
3. launcher 校验组件并使用 `tdl extension install --force` 刷新扩展。
4. API 与静态 UI server 分别绑定随机端口。
5. 浏览器自动打开本次 UI 地址。

关闭 Slint launcher 会停止 `tdl`、扩展和 UI server。开发模式不会创建系统快捷方式或卸载项。

## 扩展启动方式

`tdl-web.exe` 是扩展工作进程，不是独立的命令行程序。直接运行它会因为缺少 tdl 注入的 `TDL_EXTENSION` 环境而退出：

```powershell
.\bin\tdl-web.exe
```

正确方式是先安装扩展，再通过官方 tdl 启动：

```powershell
tdl extension install --force .\bin\tdl-web.exe
tdl web
```

扩展负责 API、Telegram 登录、账号切换、消息解析、缓存和下载；前端负责页面展示和交互。

手动执行普通 `tdl web` 时 API 仍默认监听 `127.0.0.1:8080`；只有 launcher 会通过扩展环境启用随机端口与 ready 文件协议。

## 前端开发

进入 `tdl-web-ui/`：

```powershell
cd tdl-web-ui
npm ci
$env:VITE_API_BASE = "http://127.0.0.1:8080"
npm run dev
```

前端也可以单独构建：

```powershell
npm run build
```

## 构建离线安装器

开发构建使用 `.\scripts\build-client.ps1 -Version 0.1.0`，输出 `bin\tdl-web-setup-windows-x64.exe` 和 `bin\release-manifest.json`。

正式发布追加 `-RequireOfficialTdl`；构建脚本会确认官方 `9359aa2` gitlink、`Cargo.lock` 和 launcher 版本一致。Go 产物使用 `-trimpath -buildvcs=false -ldflags "-s -w -buildid="` 去除本机路径、VCS 状态、符号和调试信息；Rust release 产物启用符号剥离、Thin LTO 和静态 CRT，并在打包前拒绝仍导入 `VCRUNTIME*.dll` 或 `MSVCP*.dll` 的 PE 文件。

## 账号和数据

扩展账号数据默认保存在：

```text
%USERPROFILE%\.tdl\extensions\data\web
```

启动器只会刷新扩展程序文件，不会删除上述目录中的账号 session、缓存或其他数据。删除构建产物、重新构建或重新安装扩展都不会自动清理账号数据。

## CI

各仓库保持独立 CI：

- `tdl-web` 使用 `GOWORK=off` 验证 GitHub module 模式。
- `tdl-web-ui` 执行 npm 测试和生产构建。
- `tdl-workspace` 递归检出子模块，验证源码模式的 Go 构建和 UI 构建。
- Windows job 构建并校验 Rust/Slint 离线安装器、随机端口测试、组件 SHA-256 和 submodule commit。
