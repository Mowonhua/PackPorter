//! 文件职责：承载 L1-L4 资产分级规则的路径常量与默认注册表。
//! 定义范围：L1/L2/L3 路径常量、RuleRegistry 与规则访问函数；不含文件遍历逻辑。

use crate::domain::instance::{AssetLevel, AssetRule};

// ==================== 常量、枚举和类型别名 ====================

/// L1 资产相对路径表：存档、服务器列表、截图与原理图，直接复制。
pub const L1_ASSETS: &[&str] = &["saves/", "servers.dat", "screenshots/", "schematics/"];

/// L2 资产相对路径表：资源包与光影包，增量合并。
pub const L2_ASSETS: &[&str] = &["resourcepacks/", "shaderpacks/"];

/// L3 资产相对路径表：地图类与常用辅助模组的个人数据，整目录直接复制。
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

/// L4 资产相对路径表：客户端偏好文件，仅白名单字段智能合并。
pub const L4_ASSETS: &[&str] = &["options.txt"];

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：内置规则表的运行时载体，将路径常量转换为带级别与说明的规则。
 * 字段说明：entries 按级别顺序排列，保证执行顺序稳定（L1→L2→L3→L4）。
 * 约束条件：由 built_in_rules() 构造，UI 与规划器不得手工拼装。
 */
#[derive(Debug, Clone)]
pub struct RuleRegistry {
    /// 全部内置规则，按级别顺序排列。
    pub entries: Vec<AssetRule>,
}

// ==================== 函数和方法定义 ====================

/**
 * 函数职责：返回内置规则注册表，覆盖 L1/L2/L3 目录与 L4 文件。
 * 输入说明：无。
 * 输出说明：始终可用的完整规则表。
 * 实现思路：将三级路径常量表逐条映射为 AssetRule（目录条目补中文说明），
 *           再追加 L4 条目，按 L1→L2→L3→L4 顺序排列。
 */
pub fn built_in_rules() -> RuleRegistry {
    let mut entries = Vec::new();
    for path in L1_ASSETS {
        entries.push(AssetRule {
            relative_path: (*path).to_string(),
            level: AssetLevel::Direct,
            description: "安全私有资产，直接复制".to_string(),
        });
    }
    for path in L2_ASSETS {
        entries.push(AssetRule {
            relative_path: (*path).to_string(),
            level: AssetLevel::Incremental,
            description: "外围资源，增量合并（同名保留新版）".to_string(),
        });
    }
    for path in L3_ASSETS {
        entries.push(AssetRule {
            relative_path: (*path).to_string(),
            level: AssetLevel::ModData,
            description: "辅助模组数据，直接复制".to_string(),
        });
    }
    for path in L4_ASSETS {
        entries.push(AssetRule {
            relative_path: (*path).to_string(),
            level: AssetLevel::SmartMerge,
            description: "客户端偏好，白名单字段智能合并".to_string(),
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
