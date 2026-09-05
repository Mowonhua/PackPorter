//! 文件职责：承载 options 键值合并的语义模型：合并动作、白名单键族与合并结果。
//! 定义范围：MergeAction 枚举、MergeOutcome 结构与白名单匹配契约；不含解析与文件 IO。

use serde::{Deserialize, Serialize};

// ==================== 枚举和类型别名 ====================

/**
 * 结构职责：单个 options 键的合并裁决。
 * 字段说明：区分保留新值、采用旧偏好、采用旧绑定、保留未验证绑定与忽略遗留键。
 * 约束条件：同一键只能有一个裁决；KeepNew 是缺省裁决。
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeAction {
    /// 新版独有键：保留新值（含新 Mod 新增键位）。
    KeepNew,
    /// 双方同名键：白名单命中且值合法时采用旧值。
    TakeOld,
    /// 键位类 key_ 前缀：旧值优先，即使新版默认值不同。
    TakeOldBinding,
    /// 目标未列出该键位，保留旧绑定，但尚未验证目标是否支持。
    TakeUnverifiedBinding,
    /// 旧版独有且未获准迁移的键或非法偏好值：不写入新版。
    DropLegacy,
}

/**
 * 结构职责：区分初始化缺失文件与合并已有配置。
 * 字段说明：Initialize 仅用于确认目标文件不存在；Merge 用于已有文件或纯映射合并。
 * 约束条件：空文件仍属于 Merge；读取错误不得转换成 Initialize。
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptionsMergeMode {
    Initialize,
    #[default]
    Merge,
}

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：单个键的合并决策明细，供 UI 逐项展示与日志输出。
 * 字段说明：old_value/new_value 保留原始书写（含引号），便于用户识别。
 * 约束条件：action 为 KeepNew 时 old_value 可为 None；action 为 DropLegacy 时 new_value 必须为 None。
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeOutcome {
    /// options 键名，如 key_key.jump 或 fov。
    pub key: String,
    /// 本键最终采取的动作。
    pub action: MergeAction,
    /// 旧文件中的原始值，旧文件缺失该键时为 None。
    pub old_value: Option<String>,
    /// 新文件中的原始值，新文件缺失该键时为 None。
    pub new_value: Option<String>,
}

/**
 * 结构职责：一次 MergeOptions 的整体合并结果。
 * 字段说明：outcomes 保序（按新文件键序优先，旧版独有键追加在后）。
 * 约束条件：merged 可序列化为 options 文本；未验证绑定不代表目标版本支持该键位。
 */
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeResult {
    /// 文件读取边界确认的处理模式；纯映射调用不推断文件存在性。
    #[serde(default)]
    pub mode: OptionsMergeMode,
    /// 逐键决策明细。
    pub outcomes: Vec<MergeOutcome>,
    /// 需要写入新文件的完整键值对（已合并）。
    pub merged: Vec<(String, String)>,
    /// 被智能忽略的旧版键数量。
    pub dropped: usize,
}

// ==================== 接口和抽象契约 ====================

/**
 * 接口职责：抽象"哪些键允许采用旧值"的白名单策略。
 * 调用方：OptionsMergeEngine 依赖它裁决每个同名键；扩展键族时新增实现。
 * 实现要求：classify 必须是纯函数，不读取文件系统；未识别键一律返回 LegacyDrop 候选。
 */
pub trait MergePolicy: Send + Sync {
    /**
     * 函数职责：判定旧版键在新版语境下的合并倾向。
     * 输入说明：key 为 options 键名；new_exists 表示新版文件是否已有该键。
     * 输出说明：返回倾向动作；引擎可基于值合法性二次否决（如数值解析失败回退 KeepNew）。
     * 实现思路：前缀匹配 key_ 判定键位族，再匹配音量/视角/语言/画质白名单前缀表。
     */
    fn classify(&self, key: &str, new_exists: bool) -> MergeAction;
}
