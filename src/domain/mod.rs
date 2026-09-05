//! 文件职责：领域层模块入口，按职责拆分数据结构、迁移规则、合并语义与事务契约。
//! 定义范围：子模块导出，不承载任何行为。

/// 领域错误类型与统一结果别名。
pub mod error;
/// 领域数据结构、错误与仓储契约。
pub mod instance;
/// 启动器进程识别与多启动器会话生命周期策略。
pub mod launcher_lifecycle;
/// 资产分级规则（L1-L4）与规则注册表。
pub mod rules;
/// options 键值合并语义：决策枚举、白名单与合并结果。
pub mod merge;
/// 迁移事务契约：动作清单、回滚动作与事务接口。
pub mod transaction;
