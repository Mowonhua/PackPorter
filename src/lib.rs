//! 文件职责：Packporter 库入口，声明领域层、服务层、基础设施层模块。
//! 定义范围：模块树与公开导出；分层依赖方向为 services → domain，infra 被 services 复用。

/// 应用配置持久化。
pub mod app_config;
/// 领域层：数据结构、规则、合并语义与事务契约。
pub mod domain;
/// 基础设施层：文件、进程、压缩与监控的具体实现。
pub mod infra;
/// 服务层：四大核心服务与迁移编排。
pub mod services;
