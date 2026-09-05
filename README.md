<p align="center">
  <img src="assets/icons/packporter.png" width="104" height="104" alt="PackPorter 草地方块包裹与迁移箭头图标">
</p>

<h1 align="center">PackPorter</h1>

<p align="center"><strong>简体中文</strong> · <a href="README.en.md">English</a></p>

<p align="center"><strong>整合包换新，带上你的世界。</strong></p>
<p align="center">Minecraft Java 版整合包迁移工具 · 存档、设置、路标与模组数据</p>

<p align="center">
  <a href="#开始使用">开始使用</a> ·
  <a href="#迁移哪些内容">迁移内容</a> ·
  <a href="#备份与边界">备份与边界</a> ·
  <a href="#开发">开发</a>
</p>

---

升级整合包时，个人进度值得保留，新版配置也需要照顾。PackPorter 按资产类型选择复制、增量保留或白名单合并，先展示迁移计划，再由你确认执行。

<p align="center">
  <img src="assets/readme/workflow.svg" width="100%" alt="迁移流程示意：选择旧、新实例，预览迁移计划，确认后迁移个人资产。">
</p>

- **先看计划**：选择源实例和目标实例后，自动列出各项资产及复制、保留、合并统计。
- **保留使用习惯**：合并允许迁移的键位、音量、画面与语言偏好，保留新版新增键位。
- **按需配置**：四级迁移内容可独立开关，路径可增删改，也可逐条启用或禁用。

## 开始使用

从 [Releases](https://github.com/Mowonhua/PackPorter/releases) 下载 Windows x64 压缩包，解压后运行 `packporter.exe`。首版 `v0.1.0-alpha.1` 为预发布版本；下载页附有 SHA-256 校验文件。

也可从源码构建运行。以下以 **Windows** 为例，需要 Rust/Cargo，以及对应工具链的链接器和 Windows 资源编译工具。

在仓库根目录运行：

```powershell
cargo run
```

1. 打开右上角**设置**，选择启动器的 `.minecraft/versions` 目录并保存。
2. 返回主页，选择旧版作为**源实例**、新版作为**目标实例**。需要刷新列表时点击扫描。
3. 检查自动生成的**迁移计划**，按需在设置中调整迁移内容和备份开关。
4. 关闭相关游戏进程，点击**开始迁移**，查看结果；可通过**打开备份目录**检查备份文件。

需要独立可执行文件时：

```powershell
cargo build --release
```

Windows 产物为 `target/release/packporter.exe`。应用图标已嵌入程序，无需额外分发图片。

## 迁移哪些内容

以下是首次使用的默认规则；在设置页各级的「配置」入口可修改路径。

| 资产 | 处理方式 |
| --- | --- |
| **L1 · 个人文件** | 存档、服务器列表、截图、原理图：直接复制 |
| **L2 · 资源与光影** | 只补入目标缺少的文件，同路径保留新版 |
| **L3 · 模组数据** | 地图、路标等个人数据：按目录复制 |
| **L4 · 个人设置** | `options.txt`：按白名单合并键值 |

例如，旧版已有的资源包遇到目标中的同路径文件时会保留目标版本；`options.txt` 中允许迁移的个人偏好则优先使用旧值。L1/L3 会覆盖同路径文件，但不会删除目标独有文件。

<details>
<summary><strong>展开：默认路径示例</strong></summary>

- **L1**：`saves/`、`servers.dat`、`screenshots/`、`schematics/`。
- **L2**：`resourcepacks/`、`shaderpacks/`。
- **L3**：`xaero/`、`journeymap/`、`config/xaero/`、`config/jei/world/`、`local/` 等。
- **L4**：`options.txt`。

</details>

<details>
<summary><strong>展开：个人设置如何合并</strong></summary>

- **首次初始化**：目标 `options.txt` 不存在时，只生成允许迁移的偏好和键位，无需先启动新整合包。已有空文件仍按合并处理。
- **读取失败**：只有 `NotFound` 会进入初始化；权限、编码等其他读取错误会使计划失败。
- **键位 `key_*`**：同名采用旧值；目标没有列出的旧键位仍保留，并计为「未验证键位」。新版新增键位保持原样。
- **个人偏好**：音量 `soundCategory_*`、`fov`、`gamma`、`guiScale`、`renderDistance`、`maxFps`、语言等白名单键优先采用旧值；旧版独有且值合法时补入目标。
- **其他旧键**：不在允许规则中的旧键不会迁入。
- **格式校验**：部分数值偏好有基础格式检查；校验不通过时保留目标已有值，目标缺少该键时不写入。检查不保证取值范围或跨版本格式兼容。
- **预览信息**：显示初始化或合并模式、设置总数及未验证键位数。

未验证键位不保证目标游戏或 Mod 支持；跨 Minecraft 版本的键名与格式转换尚未实现。

</details>

## 备份与边界

**自动备份默认开启，可在设置中关闭。** 开启后，写入前会将即将被覆盖的目标既有文件保存到目标实例的 `backups/<时间戳>-pre-migrate.zip`，它不是整个实例的完整备份。没有既有文件需要覆盖时，不生成 Zip。

迁移执行失败时，会尝试按相反顺序恢复覆盖项、删除新建项。补偿依赖进程内存，可能部分失败，不提供进程崩溃后的自动恢复保证；请以实际结果报告为准。

- **实例占用**：检测到相关 Java 游戏进程时阻断迁移；检测依据进程命令行中的实例路径，不等同于操作系统文件锁。
- **实例识别**：解析版本 JSON 和继承链，识别 NeoForge、Forge、Fabric、Quilt；支持读取 PCL2/HMCL 的 `clientVersion` 字段。
- **兼容范围**：识别 Minecraft 与 Loader 版本不代表存档或模组数据可以跨版本兼容。工具负责资产迁移，不转换存档或 Mod 数据格式。

## 开发

GitHub Actions 在推送到 `main`、提交 PR 或手动触发时运行测试并构建 Windows x64 发布包，可在 Actions 运行页的 Artifacts 中下载（保留 14 天）。CI 使用 Rust 1.93.0 和 `Cargo.lock` 中锁定的依赖。

发布时同步更新 `Cargo.toml`、`Cargo.lock` 的版本，再推送对应的 `v<版本号>` tag。测试和构建通过后自动创建 GitHub Release 并上传压缩包与校验文件；带 `-alpha`、`-beta` 或 `-rc` 等后缀的版本标记为预发布。可在 `docs/releases/<tag>.md` 提供发布说明，缺省时由 GitHub 自动生成。

**Rust + Slint**，按领域、服务、基础设施分层：领域层不做 IO，服务层编排迁移，基础设施层处理文件、进程与目录监控。

```text
src/
  domain/        数据结构、分级规则、合并语义与事务契约
  services/      实例扫描、设置合并、备份、目录监控与迁移编排
  infra/         JSON 解析、文件操作、进程探测、Zip 与目录监控
  app_config.rs  用户配置持久化
  main.rs        Slint UI 装配
ui/              界面与动效
tests/           集成与 UI 回调测试
examples/        只读扫描示例与备份基准
```

```powershell
cargo test

# 对实际 versions 目录执行只读扫描
cargo run --example smoke_real -- "E:\你的\.minecraft\versions"
```

`smoke_real` 的合并预览仅在实例目录名分别包含 `0.9.3` 和 `0.9.6` 时执行，否则跳过该部分。

<details>
<summary><strong>展开：配置与目录监控</strong></summary>

Windows 用户配置保存在 `%APPDATA%/packporter/config.json`，包含实例目录、最近选择、迁移开关与自定义路径规则。配置采用原子写入；缺少 `rules` 字段时回退到内置默认值，计划按配置中的规则生成。

`FolderWatcherService` 监控 `versions/` 的新目录事件，以 800 ms 间隔连续三轮快照一致作为目录稳定信号，再通知 UI。这是启发式判断，不代表对解压程序完成状态的直接确认。

</details>

开发细节见[项目经验](docs/experience.md)，图标素材与生成说明见[应用图标](assets/icons/README.md)。

## 许可

[MIT License](LICENSE) · Copyright © 2026 Mowon
