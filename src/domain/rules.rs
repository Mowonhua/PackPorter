//! 文件职责：承载 L1-L4 资产分级规则的路径默认值与规则语义辅助（规范化/校验/冲突）。
//! 定义范围：L1/L2/L3/L4 默认路径常量、RuleRegistry、规则访问函数与路径校验；
//!           不含文件遍历逻辑。运行时规则表来自用户配置，常量仅作首次初始化默认值。

use crate::domain::instance::{AssetLevel, AssetRule};

// ==================== 常量、枚举和类型别名 ====================

/// L1 默认路径表：存档、服务器列表、截图与原理图，直接复制。
pub const L1_ASSETS: &[&str] = &["saves/", "servers.dat", "screenshots/", "schematics/"];

/// L2 默认路径表：资源包与光影包，增量合并。
pub const L2_ASSETS: &[&str] = &["resourcepacks/", "shaderpacks/"];

/// L3 默认路径表：地图类与常用辅助模组的个人数据，整目录直接复制。
pub const L3_ASSETS: &[&str] = &[
    "xaero/",
    "journeymap/",
    "config/xaero/",
    "config/jei/world/",
    "config/journeymap/",
    "XaeroWorldMap/",
    "customdata/",
    "local/",
];

/// L4 默认路径表：客户端偏好文件，仅白名单字段智能合并。
pub const L4_ASSETS: &[&str] = &["options.txt"];

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：规则表的运行时载体，将路径条目转换为带级别与说明的规则。
 * 字段说明：entries 按级别顺序排列，保证执行顺序稳定（L1→L2→L3→L4）。
 * 约束条件：由 built_in_rules() 或配置层 effective_registry() 构造，
 *           UI 与规划器不得手工拼装。
 */
#[derive(Debug, Clone)]
pub struct RuleRegistry {
    /// 全部生效规则，按级别顺序排列。
    pub entries: Vec<AssetRule>,
}

// ==================== 函数和方法定义 ====================

/**
 * 函数职责：返回各级别的默认说明文案。
 * 输入说明：level 为资产分级。
 * 输出说明：该级别的固定中文说明。
 * 实现思路：枚举到文案的一一映射，内置规则与用户自定义规则共用。
 */
pub fn level_default_description(level: AssetLevel) -> &'static str {
    match level {
        AssetLevel::Direct => "安全私有资产，直接复制",
        AssetLevel::Incremental => "外围资源，增量合并（同名保留新版）",
        AssetLevel::ModData => "辅助模组数据，直接复制",
        AssetLevel::SmartMerge => "客户端偏好，白名单字段智能合并",
    }
}

/**
 * 函数职责：返回内置默认规则注册表（首次使用、未做自定义时的初始化值）。
 * 输入说明：无。
 * 输出说明：始终可用的完整规则表。
 * 实现思路：将四级默认路径常量逐条映射为 AssetRule（目录条目补中文说明），
 *           按 L1→L2→L3→L4 顺序排列。
 */
pub fn built_in_rules() -> RuleRegistry {
    let mut entries = Vec::new();
    for path in L1_ASSETS {
        entries.push(AssetRule {
            relative_path: (*path).to_string(),
            level: AssetLevel::Direct,
            description: level_default_description(AssetLevel::Direct).to_string(),
        });
    }
    for path in L2_ASSETS {
        entries.push(AssetRule {
            relative_path: (*path).to_string(),
            level: AssetLevel::Incremental,
            description: level_default_description(AssetLevel::Incremental).to_string(),
        });
    }
    for path in L3_ASSETS {
        entries.push(AssetRule {
            relative_path: (*path).to_string(),
            level: AssetLevel::ModData,
            description: level_default_description(AssetLevel::ModData).to_string(),
        });
    }
    for path in L4_ASSETS {
        entries.push(AssetRule {
            relative_path: (*path).to_string(),
            level: AssetLevel::SmartMerge,
            description: level_default_description(AssetLevel::SmartMerge).to_string(),
        });
    }
    RuleRegistry { entries }
}

/**
 * 函数职责：为指定相对路径查找匹配的迁移规则。
 * 输入说明：registry 为内置或扩展规则表；relative_path 为实例根目录下的相对路径，
 *           目录条目按前缀匹配，文件条目按全等匹配。
 * 输出说明：命中返回规则拷贝；未命中返回 None（调用方应跳过该路径）。
 * 实现思路：先匹配目录条目（前缀一致），再匹配文件条目（全等）。
 */
pub fn find_rule(registry: &RuleRegistry, relative_path: &str) -> Option<AssetRule> {
    let normalized = relative_path.replace('\\', "/");
    registry
        .entries
        .iter()
        .find(|rule| rule.relative_path.ends_with('/') && normalized.starts_with(&rule.relative_path))
        .or_else(|| {
            registry
                .entries
                .iter()
                .find(|rule| !rule.relative_path.ends_with('/') && rule.relative_path == normalized)
        })
        .cloned()
}

/**
 * 函数职责：规范化用户输入的规则路径并校验合法性。
 * 输入说明：raw 为用户原始输入（容忍首尾空白、反斜杠分隔符、./ 前缀）。
 * 输出说明：成功返回规范化路径（目录以 '/' 结尾，文件不带尾斜杠）；
 *           非法（空串、绝对路径、含 .. 跳级）返回中文错误说明。
 * 实现思路：统一分隔符并折叠重复斜杠 → 剥离引导斜杠 → 逐段校验 →
 *           目录判定：原输入以 '/' 结尾，或末段无扩展名点号（如 saves 视为 saves/）。
 */
pub fn normalize_rule_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_matches('"');
    if trimmed.is_empty() {
        return Err("路径不能为空。".to_string());
    }
    let mut s = trimmed.replace('\\', "/");
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    if let Some(stripped) = s.strip_prefix("./") {
        s = stripped.to_string();
    }
    let s = s.strip_prefix('/').unwrap_or(&s).to_string();
    if s.is_empty() || s == "/" {
        return Err("路径必须相对实例根目录，不能指向根目录本身。".to_string());
    }
    let segments: Vec<&str> = s.split('/').filter(|seg| !seg.is_empty()).collect();
    if segments.contains(&"..") {
        return Err("路径不允许包含 .. 跳级。".to_string());
    }
    if segments
        .first()
        .map(|seg| seg.len() >= 2 && seg.as_bytes()[1] == b':')
        .unwrap_or(false)
    {
        return Err("路径必须相对实例根目录，不能是绝对路径。".to_string());
    }
    let ends_with_slash = trimmed.replace('\\', "/").ends_with('/');
    let body = segments.join("/");
    let last = segments.last().copied().unwrap_or_default();
    // 目录判定：显式尾斜杠优先，其次按末段是否含点号启发（MC 实例根下文件普遍带扩展名）。
    let is_dir = ends_with_slash || !last.contains('.');
    Ok(if is_dir { format!("{body}/") } else { body })
}

/**
 * 函数职责：判断两条规则路径是否互相覆盖（含相等、目录前缀包含）。
 * 输入说明：a/b 为已规范化的规则路径。
 * 输出说明：冲突返回 true（同路径，或任一为目录且是另一条的前缀）。
 * 实现思路：规划器按注册表顺序首条命中即止，互相覆盖会让其中一条永不生效，
 *           故在编辑入口直接拒绝此类组合。
 */
pub fn rules_conflict(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    (a.ends_with('/') && b.starts_with(a)) || (b.ends_with('/') && a.starts_with(b))
}
