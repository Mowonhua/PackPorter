//! 文件职责：服务层模块入口，声明四个核心服务与编排入口。
//! 定义范围：模块导出；服务之间只依赖领域抽象，不互相依赖具体实现。

/// 模块 A：实例与版本探测器。
pub mod instance_service;
/// 模块 B：智能文本合并引擎。
pub mod options_merge;
/// 模块 C：备份与事务回滚引擎。
pub mod backup_engine;
/// 模块 D：启动器目录监控感知器。
pub mod folder_watcher;
/// 迁移总编排：规划 + 执行 + 进度上报。
pub mod migration_service;
