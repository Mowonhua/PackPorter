//! 文件职责：领域层错误类型，统一描述迁移流程中可能出现的失败形态。
//! 定义范围：错误枚举与从基础设施错误到领域错误的转换契约。

use thiserror::Error;

/// 版本目录根路径不可用（不存在或无读取权限）。
#[derive(Debug, Error)]
#[error("版本目录不可用: {0}")]
pub struct RootUnavailable(pub String);

#[derive(Debug, Error)]
pub enum PackError {
    /// 路径不存在、不可访问或不是目录。
    #[error("路径不可用: {0}")]
    PathUnavailable(String),

    /// JSON 解析失败，携带文件路径与底层消息。
    #[error("JSON 解析失败 [{path}]: {message}")]
    JsonParse {
        /// 解析失败的文件绝对路径。
        path: String,
        /// serde 底层错误消息。
        message: String,
    },

    /// 版本 profile 中缺失关键字段，无法确定 MC 版本或继承链。
    #[error("版本元数据不完整 [{profile_path}]: {field}")]
    ProfileIncomplete {
        /// 出错的 profile json 路径。
        profile_path: String,
        /// 缺失或非法的字段名。
        field: String,
    },

    /// 检测到运行中的游戏进程占用目标实例，迁移被阻断。
    #[error("实例被运行中的进程占用: {instance_name} (PID {pid}, {process_name})")]
    InstanceLocked {
        /// 被占用的实例（版本）名称。
        instance_name: String,
        /// 占用进程的系统 PID。
        pid: u32,
        /// 占用进程名，如 javaw.exe。
        process_name: String,
    },

    /// 迁移事务已回滚，携带回滚报告供 UI 呈现。
    #[error("迁移已回滚: {reason}")]
    RolledBack {
        /// 触发回滚的原始错误描述。
        reason: String,
        /// 回滚执行报告。
        report: String,
    },

    /// 文件系统操作失败，携带操作语义与底层消息。
    #[error("文件操作失败 [{operation} {path}]: {message}")]
    FileSystem {
        /// 操作语义，如 copy、delete、rename。
        operation: String,
        /// 目标路径。
        path: String,
        /// io 底层错误消息。
        message: String,
    },

    /// Zip 备份创建失败。
    #[error("备份失败: {0}")]
    Backup(String),

    /// 迁移计划的输入参数不满足前置条件（如源实例与目标实例相同）。
    #[error("迁移参数无效: {0}")]
    InvalidPlan(String),
}

impl From<std::io::Error> for PackError {
    /// 将 io 错误转换为领域文件错误，operation 由调用方通过错误携带的路径推断为 "io"。
    fn from(value: std::io::Error) -> Self {
        PackError::FileSystem {
            operation: "io".to_string(),
            path: String::new(),
            message: value.to_string(),
        }
    }
}

// ==================== 类型别名 ====================

/// 全库统一结果类型，错误固定为 [`PackError`]。
pub type PackResult<T> = Result<T, PackError>;
