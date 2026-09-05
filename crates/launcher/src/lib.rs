//! 文件职责：提供 GUI 与独立 shim 共用的启动器能力。
//! 定义范围：配置位置与跟随开关、受管绑定、启动参数、会话查询和进程识别。
//! 约束条件：不得依赖 GUI 主包、Slint 或迁移规则；配置写入和窗口策略由 GUI 负责。

pub mod binding;
pub mod process;
pub mod settings;
pub mod shim;
