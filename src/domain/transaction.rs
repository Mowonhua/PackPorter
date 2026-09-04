//! 文件职责：定义迁移事务契约：原子的文件动作清单、补偿动作与事务接口。
//! 定义范围：TransactionAction、UndoAction 与 MigrationTransaction 抽象；不含实现。

use crate::domain::error::PackResult;

// ==================== 枚举和类型别名 ====================

/**
 * 结构职责：事务中单个文件级动作。
 * 字段说明：动作在执行阶段按顺序应用；目录级复制由规划器展开为逐文件动作，
 *           保证补偿粒度与进度粒度一致。
 * 约束条件：source 与 destination 均为绝对路径；动作之间不得有隐式顺序依赖。
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionAction {
    /// 复制单个文件：source → destination（destination 已存在时覆盖）。
    CopyFile {
        /// 来源文件绝对路径。
        source: std::path::PathBuf,
        /// 目标文件绝对路径。
        destination: std::path::PathBuf,
    },
    /// 用新内容覆盖写入目标文本文件。
    WriteText {
        /// 目标文件绝对路径。
        destination: std::path::PathBuf,
        /// 完整新文本内容。
        content: String,
    },
}

/**
 * 结构职责：补偿（回滚）动作，与已执行的事务动作一一对应。
 * 字段说明：Delete 用于执行阶段新建的路径；Restore 内联保存迁移前快照字节，
 *           不依赖外部备份文件，保证回滚自包含。
 * 约束条件：回滚按执行逆序应用；单条补偿失败不中断整体回滚，错误记入报告。
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoAction {
    /// 删除执行阶段新建的路径（文件或目录树）。
    Delete {
        /// 需要删除的绝对路径。
        path: std::path::PathBuf,
    },
    /// 用迁移前快照字节还原被覆盖的目标文件。
    Restore {
        /// 原位置绝对路径。
        original_path: std::path::PathBuf,
        /// 覆盖前的文件内容快照。
        content: Vec<u8>,
    },
}

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：事务回滚后的报告。
 * 字段说明：restored 与 failed 计数之和等于尝试的补偿动作总数。
 * 约束条件：failed 不为 0 时 log 必须包含每条失败的路径与原因。
 */
#[derive(Debug, Clone, Default)]
pub struct RollbackReport {
    /// 成功还原/删除的补偿动作数。
    pub restored: usize,
    /// 失败的补偿动作数。
    pub failed: usize,
    /// 回滚过程日志（含失败原因）。
    pub log: String,
}

// ==================== 接口和抽象契约 ====================

/**
 * 接口职责：抽象"以事务语义执行文件动作"的能力，是回滚保证的核心契约。
 * 调用方：MigrationService 依赖它执行计划；BackupEngine 提供默认实现。
 * 实现要求：任一动作失败必须停止执行并自动回滚全部已完成动作；成功时提交。
 */
pub trait MigrationTransaction: Send + Sync {
    /**
     * 函数职责：在一个事务内顺序执行动作清单，失败自动回滚。
     * 输入说明：actions 为规划器产出的动作清单，顺序即执行顺序；progress 为进度回调。
     * 输出说明：全部成功返回应用数量；任何失败返回 RolledBack 错误并携带回滚报告。
     * 实现思路：执行前对将被覆盖的既有文件做内存快照，逐动作执行并登记补偿，
     *           失败时逆序补偿（删除新建项 / 还原覆盖项），恢复到迁移前状态。
     */
    fn execute(
        &self,
        actions: &[TransactionAction],
        progress: &mut dyn FnMut(crate::domain::instance::MigrationProgress),
    ) -> PackResult<usize>;
}
