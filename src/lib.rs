//! 文件职责：Packporter 库入口，声明应用控制器、配置、领域、服务与基础设施模块。
//! 定义范围：模块树与公开导出；分层依赖方向为 services → domain，infra 被 services 复用。
//! 约束条件：Slint 生成类型必须且只能在本 crate 实例化一次，bin 与测试统一引用此处导出。

slint::include_modules!();

/// 应用交互控制器：UI 回调到服务层的装配（供入口与集成测试复用）。
pub mod app_controller;
/// 应用配置持久化。
pub mod app_config;
/// 领域层：数据结构、规则、合并语义与事务契约。
pub mod domain;
/// 基础设施层：文件、进程、压缩与监控的具体实现。
pub mod infra;
/// 服务层：四大核心服务与迁移编排。
pub mod services;
