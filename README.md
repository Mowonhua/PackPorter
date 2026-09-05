# PackPorter

Minecraft JAVA 版整合包平滑迁移工具。升级整合包新版本时，将旧版本的个人游戏资产（存档、个人设置、路标、模组数据等）安全迁移到新版本，杜绝"盲目覆盖导致新版本平衡性配置崩溃"。

## 资产分级策略

| 级别 | 默认目标 | 策略 |
| --- | --- | --- |
| L1 Direct | `saves/`、`servers.dat`、`screenshots/`、`schematics/` | 直接复制 |
| L2 Incremental | `resourcepacks/`、`shaderpacks/` | 增量合并：旧有新缺才复制，同名保留新版 |
| L3 ModData | `xaero/`、`journeymap/`、`config/xaero/`、`config/jei/world/`、`local/` 等 | 整目录直接复制 |
| L4 SmartMerge | `options.txt` | 白名单智能合并，严禁整文件覆盖 |

上表路径为**首次使用的默认值**：设置页每级内容行末尾的「配置」入口可对各级路径增删改、逐条启用/禁用，自定义规则持久化到用户配置，生成计划时代码不依赖任何硬编码路径。

L4 合并语义（`OptionsMergeEngine`）：

- `key_` 键位族：旧值优先；新版文件存在且键位族非空时，旧版已淘汰键位（改名/移除）智能忽略。
- 音量（`soundCategory_*`）、视角画质（`fov`/`gamma`/`guiScale`/`renderDistance`/`maxFps`）、语言等偏好键：同名采用旧值；旧版独有且值合法时补入新版（options.txt 只存非默认值）。
- 新 Mod 新增键位：原样保留。
- 其余旧版遗留键（旧 Mod 残留）：智能忽略。
- 数值/布尔键做合法性校验，脏值回退新版默认。

## 技术栈与模块

Rust + Slint 桌面应用，三层架构（领域层无 IO，服务层编排，基础设施层落地）：

- **模块 A `InstanceService`**：扫描 `versions/`，解析版本 json（含 `inheritsFrom` 继承链、PCL2/HMCL 的 `clientVersion` 字段），识别 MC 版本与 Loader（NeoForge/Forge/Fabric/Quilt 及版本号）；通过 sysinfo 枚举 java 进程检测实例占用，占用即抛 `InstanceLocked` 阻断迁移。
- **模块 B `OptionsMergeEngine`**：`merge_options(old, new)` 解析为键值映射后按白名单+规则合并；`merge_maps` 纯函数实现供测试与 UI 预览复用。
- **模块 C `BackupEngine`**：写前对将被覆盖的既有文件做增量 Zip 镜像（`backups/<时间戳>-pre-migrate.zip`）；`MigrationTransaction` 实现类事务——执行前内存快照，逐动作登记补偿（Delete 新建项 / Restore 覆盖项），任一失败立即逆序回滚并返回 `RolledBack` 报告。
- **模块 D `FolderWatcherService`**：notify 监控 `versions/` 新目录事件，SnapshotProbe 连续 3 轮（800ms 间隔）快照一致判定"解压完成"，事件经 mpsc + `slint::invoke_from_event_loop` 唤起 UI。
- **配置持久化 `AppConfig`**：`%APPDATA%/packporter/config.json`，记录 versions 路径、最近选择、各级迁移开关与自定义迁移规则（`rules` 缺省回退内置默认），原子写防止配置损坏。

## 构建与运行

```powershell
cargo run                      # 启动 GUI
cargo test                     # 23 项单元/集成测试（含端到端迁移）+ UI 回调集成测试
cargo run --example smoke_real -- "E:\你的\.minecraft\versions"   # 对真实目录只读扫描 + 合并预览
cargo build --release
```

## 目录结构

```
src/
  domain/       # 领域层：数据结构、L1-L4 规则、合并语义、事务契约
  services/     # 服务层：InstanceService / OptionsMergeEngine / BackupEngine / FolderWatcherService / MigrationService
  infra/        # 基础设施层：json 解析、key:value 合并策略、进程探测、zip、目录监控
  app_config.rs # 配置持久化
  main.rs       # Slint UI 装配
ui/packporter.slint
tests/merge_engine.rs
```
