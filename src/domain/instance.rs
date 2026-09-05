//! 文件职责：领域层数据结构与仓储契约，全部为纯数据与抽象接口，不含 IO 实现。
//! 定义范围：MinecraftVersion、LoaderKind、InstanceProfile、AssetRule、MergeDecision、
//!           MigrationPlan、迁移进度事件、事务结果与版本 profile 读取契约。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::domain::error::PackResult;

// ==================== 枚举和类型别名 ====================

/**
 * 结构职责：资产迁移的四级策略分级，决定执行引擎对每条资产的行为。
 * 字段说明：枚举项与产品需求中的 L1-L4 一一对应。
 * 约束条件：执行器必须对每个枚举项有明确分支；新增级别需同步执行引擎。
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetLevel {
    /// L1 安全私有资产：直接复制，冲突时旧文件覆盖新文件。
    Direct,
    /// L2 外围资源：增量合并，旧有新缺才复制，同名一律保留新版。
    Incremental,
    /// L3 辅助模组数据：整目录直接复制，冲突时旧文件覆盖。
    ModData,
    /// L4 客户端偏好：仅按白名单字段智能合并，严禁整文件覆盖。
    SmartMerge,
}

impl AssetLevel {
    /**
     * 函数职责：给出级别的展示序号（L1=1 … L4=4）。
     * 输入说明：无。
     * 输出说明：1-4 的级别序号。
     * 实现思路：枚举项到序号的一一映射，供配置与 UI 以整数索引级别。
     */
    pub fn index(self) -> usize {
        match self {
            AssetLevel::Direct => 1,
            AssetLevel::Incremental => 2,
            AssetLevel::ModData => 3,
            AssetLevel::SmartMerge => 4,
        }
    }

    /**
     * 函数职责：按序号反查级别。
     * 输入说明：index 为 1-4 的级别序号。
     * 输出说明：命中返回级别；越界返回 None。
     * 实现思路：序号到枚举项的映射，与 index() 互逆。
     */
    pub fn from_index(index: u32) -> Option<Self> {
        match index {
            1 => Some(AssetLevel::Direct),
            2 => Some(AssetLevel::Incremental),
            3 => Some(AssetLevel::ModData),
            4 => Some(AssetLevel::SmartMerge),
            _ => None,
        }
    }
}

/**
 * 结构职责：复制型资产（L1/L2/L3）中单个文件条目的执行决策。
 * 字段说明：由规划器扫描源/目标目录对比后产出；L4 条目不产生本结构（用 MergeOutcome）。
 * 约束条件：action 为 CopyFromOld 时源文件必须存在。
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeDecision {
    /// 实例根目录下的相对路径（文件）。
    pub relative_path: String,
    /// 对该路径采取的动作。
    pub action: DecisionAction,
}

/**
 * 结构职责：MergeDecision 的动作枚举。
 * 字段说明：覆盖复制型资产的全部可能裁决。
 * 约束条件：无。
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionAction {
    /// 旧版存在且可迁移：从旧实例复制。
    CopyFromOld,
    /// 同名冲突且策略要求保留新版：跳过复制。
    KeepNew,
    /// 旧版缺失该路径：无动作。
    SourceMissing,
}

/**
 * 结构职责：标识实例所用的模组加载器家族。
 * 字段说明：Vanilla 表示 profile 缺失或无法识别，按原版实例处理。
 * 约束条件：比较与展示均不区分大小写；不承载版本号信息。
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoaderKind {
    /// 原版或无法识别的加载器。
    Vanilla,
    /// Fabric 加载器。
    Fabric,
    /// Forge 加载器。
    Forge,
    /// NeoForge 加载器。
    NeoForge,
    /// Quilt 加载器。
    Quilt,
}

/**
 * 结构职责：版本目录内唯一标识一个实例：目录名 + 关联 jar 名。
 * 字段说明：dir_name 必须与 versions/ 下真实目录名一致；jar_name 可为空表示无关联 jar。
 * 约束条件：由 InstanceService 探测产出，UI 不得手工构造。
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinecraftVersion {
    /// versions/ 下的目录名，同时是实例对外显示名。
    pub dir_name: String,
    /// 与目录关联的 <name>.jar 文件名（不含 .jar 后缀），可为空。
    pub jar_name: String,
}

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：描述一个可迁移实例的完整画像，是迁移计划与 UI 列表的数据来源。
 * 字段说明：所有路径均为绝对路径；存在性以探测时刻的文件系统状态为准。
 * 约束条件：root_dir 必须存在且为目录；locked 为 true 时迁移必须被 UI 阻断。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceProfile {
    /// 版本目录标识（目录名 + jar 名）。
    pub version: MinecraftVersion,
    /// 版本目录的绝对路径。
    pub root_dir: PathBuf,
    /// 版本 json 的绝对路径，缺失时为 None。
    pub profile_path: Option<PathBuf>,
    /// 解析后的 MC 基础版本号，无法确定时为 "unknown"。
    pub mc_version: String,
    /// 加载器家族。
    pub loader: LoaderKind,
    /// 加载器版本号，未知时为 None。
    pub loader_version: Option<String>,
    /// 是否被运行中的游戏进程占用。
    pub locked: bool,
    /// 占用进程描述（进程名 + PID），未占用时为 None。
    pub locked_by: Option<String>,
}

/**
 * 结构职责：单条资产的迁移规则，描述"哪个相对路径按哪级策略迁移"。
 * 字段说明：relative_path 为实例根目录下的相对路径，目录以 '/' 结尾便于区分文件与目录。
 * 约束条件：level 决定执行引擎采用的行为分支；description 仅供 UI 展示。
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRule {
    /// 实例根目录下的相对路径，目录条目以 '/' 结尾。
    pub relative_path: String,
    /// 分级策略，决定复制/合并/忽略行为。
    pub level: AssetLevel,
    /// 面向用户的一句说明。
    pub description: String,
}

/**
 * 结构职责：迁移计划中单个资产条目的执行决策与统计。
 * 字段说明：复制型条目（L1/L2/L3）的 decisions 逐文件记录；L4 条目 decisions 为空，
 *           明细承载于 MigrationPlan.options_results，total_items 等于合并键数。
 * 约束条件：total_items 是决策/明细数量的冗余计数，供 UI 直接显示。
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetPlanEntry {
    /// 关联的迁移规则。
    pub rule: AssetRule,
    /// 逐文件决策明细（L4 条目为空）。
    pub decisions: Vec<MergeDecision>,
    /// 决策或明细总数。
    pub total_items: usize,
}

/**
 * 结构职责：用户可选的迁移范围开关，决定规划器纳入哪些资产级别、执行器是否备份。
 * 字段说明：布尔开关缺省均为开启（保守迁移）；由设置界面编辑并随计划持久化。
 * 约束条件：计划生成时快照进 MigrationPlan，执行器只读计划中的副本，不回读实时配置。
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MigrationOptions {
    /// 是否在迁移前自动创建 Zip 备份。
    pub auto_backup: bool,
    /// 是否迁移 L1 存档等安全私有资产。
    pub include_saves: bool,
    /// 是否迁移 L2 资源包/光影包。
    pub include_packs: bool,
    /// 是否迁移 L3 模组数据。
    pub include_moddata: bool,
    /// 是否执行 L4 options 智能合并。
    pub include_options: bool,
}

impl MigrationOptions {
    /**
     * 函数职责：提供全开配置（Default 派生布尔为 false，故显式实现）。
     * 输入说明：无。
     * 输出说明：全部开关开启的选项。
     * 实现思路：逐字段赋 true。
     */
    pub fn all_enabled() -> Self {
        Self {
            auto_backup: true,
            include_saves: true,
            include_packs: true,
            include_moddata: true,
            include_options: true,
        }
    }

    /**
     * 函数职责：判断指定资产级别当前是否纳入迁移。
     * 输入说明：level 为资产分级。
     * 输出说明：级别被关闭时返回 false，其余返回 true。
     * 实现思路：级别到开关字段的映射。
     */
    pub fn allows(self, level: AssetLevel) -> bool {
        match level {
            AssetLevel::Direct => self.include_saves,
            AssetLevel::Incremental => self.include_packs,
            AssetLevel::ModData => self.include_moddata,
            AssetLevel::SmartMerge => self.include_options,
        }
    }
}

/**
 * 结构职责：单个 L4 偏好文件的合并计划：规则相对路径 + 该文件的合并结果。
 * 字段说明：relative_path 为实例根目录下的文件路径（来自 L4 规则，不硬编码）；
 *           result 承载逐键合并明细与最终键值序列。
 * 约束条件：relative_path 必须与产出它的 L4 规则一致；执行器据此定位写回目标。
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionsMergeOutcome {
    /// 关联 L4 规则的相对路径（文件）。
    pub relative_path: String,
    /// 该文件的合并结果。
    pub result: crate::domain::merge::MergeResult,
}

/**
 * 结构职责：完整迁移计划，由规划器产出、确认页消费、执行器消费。
 * 字段说明：backup_dir 在计划阶段即确定命名，但目录在执行阶段才创建；
 *           options_results 承载各 L4 文件的逐键合并明细；options 为计划时快照的迁移选项。
 * 约束条件：entries 的相对路径互不重叠；source 与 target 实例目录不得相同。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPlan {
    /// 计划生成时快照的迁移选项，执行器据此决定备份行为。
    #[serde(default = "crate::domain::instance::MigrationOptions::all_enabled")]
    pub options: MigrationOptions,
    /// 源实例（旧版本）。
    pub source: InstanceProfile,
    /// 目标实例（新版本）。
    pub target: InstanceProfile,
    /// 按规则顺序排列的资产条目。
    pub entries: Vec<AssetPlanEntry>,
    /// 执行阶段将创建的备份目录绝对路径。
    pub backup_dir: PathBuf,
    /// 各 L4 规则文件的合并明细；无 L4 条目或 L4 关闭时为空。
    #[serde(default)]
    pub options_results: Vec<OptionsMergeOutcome>,
}

/**
 * 结构职责：迁移执行过程的进度事件流单元，驱动 UI 进度条与日志区。
 * 字段说明：done 与 total 用于计算百分比；total 为 0 时进度条应显示不确定态。
 * 约束条件：事件只描述状态，UI 不得根据事件反向修改迁移数据。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationProgress {
    /// 已完成的工作单元数。
    pub done: usize,
    /// 工作单元总数，0 表示总数未知。
    pub total: usize,
    /// 当前正在处理的路径或键名，可为空。
    pub current: String,
}

/**
 * 结构职责：迁移事务的最终结果汇总。
 * 字段说明：rolled_back 为 true 时 moved_items 表示已回滚的条目数。
 * 约束条件：由执行器在事务收尾时产出一次。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionOutcome {
    /// 迁移是否成功且未回滚。
    pub success: bool,
    /// 是否发生了回滚。
    pub rolled_back: bool,
    /// 成功应用（或已回滚）的动作数。
    pub moved_items: usize,
    /// 供状态栏展示的人类可读单行报告。
    pub report: String,
}

// ==================== 接口和抽象契约 ====================

/**
 * 接口职责：抽象"读取单个版本 profile 并解析为实例画像"的能力。
 * 调用方：InstanceService 依赖它完成 versions/ 扫描；测试可注入假实现。
 * 实现要求：必须解析 inheritsFrom 继承链；profile 缺失时返回 Vanilla 兜底画像，不返回错误。
 */
pub trait VersionProfileReader: Send + Sync {
    /**
     * 函数职责：读取版本目录内所有 json，解析出 MC 版本与加载器信息。
     * 输入说明：version_dir 为 versions/ 下单个版本目录的绝对路径；dir_name 为该目录名。
     * 输出说明：成功返回完整画像；目录不可读时返回 PathUnavailable。
     * 实现思路：定位 <jar>.json，沿 inheritsFrom 向上合并元数据，按 json 关键字判定 Loader。
     */
    fn read(&self, version_dir: &std::path::Path, dir_name: &str) -> PackResult<InstanceProfile>;
}

/**
 * 结构职责：options.txt 与同类键值文本的解析结果载体。
 * 字段说明：entries 按键排序存储；value 保留原始引号书写形式。
 * 约束条件：解析必须容忍空行、注释行与重复键（后值覆盖前值）。
 */
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParsedOptions {
    /// 解析出的键值对，键为原始 key 字符串。
    pub entries: BTreeMap<String, String>,
    /// 解析过程中被跳过的非键值行（注释、空行），按出现顺序记录。
    pub skipped_lines: Vec<String>,
}

/**
 * 接口职责：抽象 options 类文本的解析能力，供合并引擎与测试解耦文件 IO。
 * 调用方：OptionsMergeEngine 依赖它；解析格式扩展（如 TOML）时新增实现而非修改引擎。
 * 实现要求：不得丢行；无法解析的行必须进入 skipped_lines 而不是报错。
 */
pub trait OptionsParser: Send + Sync {
    /**
     * 函数职责：将 options 文本解析为键值映射。
     * 输入说明：raw 为文件完整文本内容，编码假定为 UTF-8（带 BOM 需剥离）。
     * 输出说明：永远成功返回 ParsedOptions；解析失败不在此层报错。
     * 实现思路：按行拆分，以首个 ':' 分隔键值；无冒号的行进 skipped_lines。
     */
    fn parse(&self, raw: &str) -> ParsedOptions;
}

// ==================== 函数和方法定义 ====================

impl LoaderKind {
    /**
     * 函数职责：将 profile json 中的加载器线索字符串映射为加载器枚举。
     * 输入说明：hints 为从 json 文本中提取的候选关键词集合，如完整 json 小写文本或库坐标。
     * 输出说明：任一关键词命中即返回对应家族，全部未命中返回 Vanilla。
     * 实现思路：按优先级 NeoForge > Forge > Fabric > Quilt 依次匹配子串，
     *           避免 "forge" 作为 "neoforge" 子串造成的误判。
     */
    pub fn from_profile_hints(hints: &[&str]) -> LoaderKind {
        // 逐级判定：命中即返回，保证高优先级家族不被低优先级关键词覆盖。
        let contains = |needles: &[&str]| {
            hints.iter().any(|h| {
                let lowered = h.to_lowercase();
                needles.iter().any(|n| lowered.contains(n))
            })
        };
        if contains(&["neoforge", "neoforged"]) {
            LoaderKind::NeoForge
        } else if contains(&["forge"]) {
            LoaderKind::Forge
        } else if contains(&["fabric"]) {
            LoaderKind::Fabric
        } else if contains(&["quilt"]) {
            LoaderKind::Quilt
        } else {
            LoaderKind::Vanilla
        }
    }
}

impl MinecraftVersion {
    /**
     * 函数职责：给出该实例关联 jar 的完整文件名。
     * 输入说明：无。
     * 输出说明：有 jar 名时返回 "<jar_name>.jar"，否则返回 None。
     * 实现思路：对 jar_name 非空判断后拼接后缀。
     */
    pub fn jar_file_name(&self) -> Option<String> {
        if self.jar_name.is_empty() {
            None
        } else {
            Some(format!("{}.jar", self.jar_name))
        }
    }
}

impl MigrationPlan {
    /**
     * 函数职责：统计计划中全部决策数量，供执行器预分配进度条总量。
     * 输入说明：无。
     * 输出说明：所有条目 decisions 数量之和。
     * 实现思路：对 entries 逐项累加 total_items。
     */
    pub fn total_actions(&self) -> usize {
        self.entries.iter().map(|e| e.total_items).sum()
    }
}
