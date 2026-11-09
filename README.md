# 画时：ZECTRIX NOTE4 名画时钟

把 ZECTRIX NOTE4 变成一幅会换的名画。画在上方，时间、日期和画名在下方；按一下确认键，这幅画会讲自己的故事。

![四幅内置画的时钟画面](docs/images/clock.png)

- **391 幅名画，一年多不重复**：来自 Wikimedia Commons 的公有领域作品，每幅都有中文标题、作者、年代和讲解。
- **为 16 级灰阶屏挑选和调校**：自动转灰、拉伸对比度、提亮暗画、找细节最多的位置裁切；上屏发灰的画在生成目录时就筛掉了。
- **用按键看画**：确认键打开讲解，上下键换画或翻页。
- **管理网页**：手机或电脑浏览器打开，上传自己的图，选择轮播或固定一幅，调换画间隔，改写标题和讲解。
- **离线也能用**：没联网时显示 4 幅内置画，每幅都有手写中文讲解。
- **整分钟翻页**：按实测刷新耗时提前推送，数字在整分钟前后 1 秒内变化。
- **不需要任何 API key**，一个 Rust 程序运行在电脑、NAS 或其他常开的 Linux 机器上。

| 讲解页 | 管理网页 |
| --- | --- |
| ![介绍页第 1、2 页](docs/images/guide.png) | ![管理网页](docs/images/admin.png) |

## 工作方式

NOTE4 本身不运行这个程序。程序运行在同一局域网的一台电脑上，通过 [ZECTRIX 本地推送 SDK](https://github.com/itopinion/zectrix-local-push-sdk) 每分钟把画面推给设备，并接收设备上的按键。

程序断开后，设备会回到自带界面，所以程序需要一直运行。推荐放在 NAS 或其他常开的机器上。

## 准备

**设备**

- ZECTRIX Note 4（400×300），**中文固件 3.7.5**。升级方法见 SDK 的[固件说明](https://github.com/itopinion/zectrix-local-push-sdk/blob/main/docs/FIRMWARE.md)。
- 设备连上 Wi-Fi，并和运行程序的机器在同一局域网。
- 在设备上打开「设置 → 网络同步 → 联网保持」，建议常插电。
- 记下设备的局域网 IP，例如 `192.168.1.100`。可以在路由器的设备列表里查看。

**运行程序的机器**（macOS 或 Linux）

- Git
- Rust stable：<https://rustup.rs/>
- C/C++ 编译器和 CMake：SDK 会从源码编译 Opus
  - macOS：`xcode-select --install` 和 `brew install cmake`
  - Debian / Ubuntu：`sudo apt install build-essential cmake pkg-config`
  - 没有 sudo 的 Linux 机器见下文“部署到 NAS”。
- 首次编译需要联网下载 Rust 依赖。运行时需要访问 `upload.wikimedia.org` 下载名画，连不上时只显示内置画。

## 快速开始

SDK 是 Git 子模块，克隆时要带上 `--recurse-submodules`：

```sh
git clone --recurse-submodules https://github.com/FiniteLab/zectrix-note4-art-clock.git
cd zectrix-note4-art-clock/app
export CMAKE_POLICY_VERSION_MINIMUM=3.5
cargo test --locked
cargo build --release --locked
```

如果克隆时漏了子模块，`zectrix-local-push-sdk/` 是空目录，编译会报找不到 `zectrix-sdk`。在仓库根目录运行 `git submodule update --init` 补上。

`CMAKE_POLICY_VERSION_MINIMUM=3.5` 用来兼容 CMake 4 和 Opus 的旧构建脚本，用旧版 CMake 时不设也可以。

**1. 不连设备，先渲染一帧**

```sh
./target/release/art-clock --render /tmp/art-clock.png --demo
```

成功时输出 `已渲染 /tmp/art-clock.png：…`。打开这张 400×300 的 PNG，应该能看到画、时钟和右上角的电量、Wi-Fi 示例。

**2. 连接设备**

```sh
./target/release/art-clock 192.168.1.100
```

把 IP 换成自己设备的。日志出现 `已连接` 和 `显示 HH:MM，用时 …s` 时，设备上应该已经换成时钟画面。之后程序会在后台下载今天和前几天的画，下载好后自动换上。按 `Ctrl+C` 退出，设备回到自带界面。

**3. 打开管理网页**

日志里有一行 `管理网页：http://…:8686/`。用同一局域网的手机或电脑打开它。

注意：

- 程序从源码里的 `app/assets/` 读字体和名画目录。不要只把 `target/release/art-clock` 拷到别处运行；确实要拷时，加 `--assets <assets 目录>`。
- 设备同一时间只接受一个连接。另一台机器上已经在运行时，先停掉它。
- macOS 第一次运行时如果询问“查找本地网络上的设备”，请选择允许。
- 防火墙需要放行 TCP 8686（管理网页）、UDP 32343（接收设备发起的连接）和 UDP 32342（设备通信）。

## 使用

**按键**

| 按键 | 时钟画面 | 讲解页 |
| --- | --- | --- |
| 确认 短按 | 打开这幅画的讲解 | 回到时钟 |
| 上 / 下 短按 | 上一幅 / 下一幅 | 上一页 / 下一页；翻过第 1 页或最后一页回到时钟 |
| 上 或 下 长按 | 把屏幕交还设备 | 同左 |

把屏幕交还设备后，在设备上选「设置 → 网络同步 → 连接管理端」就能回到时钟。讲解页 2 分钟没有操作时，会在下一个整分钟回到时钟。

**管理网页**

- **现在**：正在显示的画、接下来几天的队列、设备连接状态，可以固定其中任意一幅。
- **我的图库**：上传自己的图片，可以删除。
- **名画目录**：搜索全部 391 幅，查看讲解，设为固定显示。
- **设置**：轮播或固定一幅；换画间隔 5 分钟到 24 小时（默认每天）；名画和自己的图按比例混合（0 表示不用这一类）；可选的管理密码。
- 每幅画都可以换图、调裁切位置、改标题、作者、年代和讲解。改动只存在自己的数据目录里。

不设密码时，同一局域网里的人都能打开管理网页。

## 命令和选项

```text
art-clock <设备 IP> [选项]            连接设备，持续显示时钟
art-clock --render <输出.png> [...]   不连设备，只渲染一帧
art-clock --sync [--days N]          只下载画作，然后退出
```

| 选项 | 作用 |
| --- | --- |
| `--cache <目录>` | 画作缓存，默认 `~/.cache/art-clock`，保留 14 天 |
| `--data <目录>` | 设置、自己的图和改过的讲解，默认 `~/.local/share/art-clock` |
| `--web <端口>` | 管理网页端口，默认 8686；也可以写成 `地址:端口` |
| `--no-web` | 不开管理网页 |
| `--offline` | 不联网，只用内置画和已缓存的画 |
| `--assets <目录>` | 素材目录，默认是源码里的 `app/assets/` |
| `--at "YYYY-MM-DD HH:MM"` | 和 `--render` 一起用，渲染指定时间 |
| `--art <N>` | 和 `--render` 一起用，渲染画廊里的第 N 幅（0 是当前的画） |
| `--guide <页>` | 和 `--render` 一起用，渲染讲解的第几页 |
| `--demo` | 和 `--render` 一起用，填入示例电量和 Wi-Fi |

## 部署到 NAS 或 Linux 常开机器

`app/deploy/nas-build.sh` 在 Linux x86_64 上编译程序，并装成 systemd 用户服务 `art-clock`：开机自启，出错 15 秒后重启。它不需要 sudo，也不需要系统 C 编译器，会用 pip 在 `~/note4-art-clock/.tools` 里装 zig 和 CMake。机器上需要先装好 Rust（`~/.cargo/bin`）和 python3。

源码必须放在 `~/note4-art-clock`：

```sh
git clone --recurse-submodules https://github.com/FiniteLab/zectrix-note4-art-clock.git ~/note4-art-clock
sh ~/note4-art-clock/app/deploy/nas-build.sh 192.168.1.100
```

也可以在 Mac 上一条命令同步源码到 NAS，并在 NAS 上编译、重启服务（第二个参数是 `~/.ssh/config` 里的主机名，默认 `nas`）：

```sh
sh app/deploy/nas.sh 192.168.1.100 nas
```

常用命令：

```sh
systemctl --user status art-clock     # 查看状态
systemctl --user stop art-clock       # 停止
tail -f ~/note4-art-clock/art-clock.log
```

## 交给 AI 安装

可以把下面这段话交给自己的 AI 助手：

> 按 https://github.com/FiniteLab/zectrix-note4-art-clock 的 README 安装“画时”。我的 NOTE4 设备 IP 是 `<设备 IP>`，已刷中文固件 3.7.5 并打开“联网保持”。用 `--recurse-submodules` 克隆，缺 Rust、C 编译器或 CMake 时先装好；在 `app/` 里运行 `cargo test --locked` 和 `cargo build --release --locked`。先用 `--render /tmp/art-clock.png --demo` 渲染一帧并确认图片正常，再连接设备，看到日志里的“已连接”和“显示 HH:MM”后告诉我管理网页地址。不要刷固件，不要连接其他设备。

## 项目结构

```text
app/                       画时程序（Rust）
  src/bin/art-clock.rs     主程序：整分钟推送、按键、讲解页、后台取画、断线重连
  src/bin/catalog.rs       在开发机上重新生成名画目录（约 15 分钟）
  src/web.rs, admin.html   管理网页
  src/layout.rs            时钟和讲解页版式
  src/art.rs               转灰、调对比度、裁切
  src/library.rs           每日选画、下载、缓存
  assets/catalog.json      391 幅名画目录
  assets/art/              4 幅内置画
  assets/fonts/            字体和授权文件
  deploy/                  NAS 部署脚本和 systemd 单元
zectrix-local-push-sdk/    ZECTRIX 本地推送 SDK（Git 子模块）
docs/images/               README 截图
mockups/                   早期草图和字体测试
DESIGN.md                  设计说明和实机测试记录
```

设计取舍、实机测试数据和名画筛选规则见 [DESIGN.md](DESIGN.md)。

## 素材来源与许可

- 名画：Wikimedia Commons 上的公有领域图片，运行时下载，不随仓库分发。作品信息来自 Wikidata。
- 讲解：摘自中文维基百科，按 CC BY-SA 4.0 使用，讲解页注明出处条目。
- 内置画：芝加哥艺术博物馆 CC0 作品，中文讲解为本项目撰写。
- 字体：GFS Didot（OFL）、思源黑体（OFL）、Fusion Pixel（OFL 等），授权文件在 `app/assets/fonts/`。
- 繁简转换字表：OpenCC `TSCharacters.txt`（Apache-2.0）。
- SDK：ZECTRIX 本地推送 SDK（MIT）。
