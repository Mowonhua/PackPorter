//! 文件职责：应用配置持久化模型：用户偏好（versions 路径、最近选择、迁移选项、迁移规则）。
//! 定义范围：AppConfig / UserRuleEntry 结构、默认值与加载/保存实现、生效规则表构造。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::domain::instance::{AssetLevel, AssetRule, MigrationOptions};
use crate::domain::rules::{level_default_description, RuleRegistry, L1_ASSETS, L2_ASSETS, L3_ASSETS, L4_ASSETS};

// ==================== 常量、枚举和类型别名 ====================

/// 配置文件名，存放于用户配置目录的 packporter 子目录下。
pub const CONFIG_FILE_NAME: &str = "config.json";

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：单条用户可编辑的迁移规则：相对路径 + 级别 + 启用开关。
 * 字段说明：relative_path 为实例根目录下的相对路径，目录以 '/' 结尾（经规范化）。
 * 约束条件：路径由 domain::rules::normalize_rule_path 校验后写入；enabled 缺省为 true。
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserRuleEntry {
    /// 实例根目录下的相对路径，目录条目以 '/' 结尾。
    pub relative_path: String,
    /// 分级策略，决定复制/合并/忽略行为。
    pub level: AssetLevel,
    /// 是否启用该规则；禁用后在生成计划时跳过。
    #[serde(default = "crate::app_config::default_true")]
    pub enabled: bool,
}

/**
 * 函数职责：serde 缺省值辅助：布尔字段缺省为 true（保守迁移）。
 * 输入说明：无。
 * 输出说明：恒为 true。
 * 实现思路：常量函数。
 */
fn default_true() -> bool {
    true
}

/**
 * 结构职责：跨会话持久的用户配置。
 * 字段说明：全部字段可缺省为默认值；反序列化容忍未知字段（向前兼容）。
 * 约束条件：versions_dir 为空表示尚未选择；rules 为 None 表示尚未自定义
 *           （使用内置默认规则），Some(空) 表示用户清空了全部规则（按空表生效）。
 */
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// .minecraft/versions 目录路径（用户选择）。
    pub versions_dir: String,
    /// 上次迁移的源实例目录名。
    pub last_source: String,
    /// 上次迁移的目标实例目录名。
    pub last_target: String,
    /// 是否在迁移前自动创建 Zip 备份（默认开启）。
    pub auto_backup: bool,
    /// 是否跟随 shim 启动的 PCL2 / HMCL 会话；旧配置缺省关闭。
    pub follow_launchers: bool,
    /// 关闭窗口时保留托盘运行；旧配置缺省关闭，只有成功保存后才生效。
    pub close_to_tray: bool,
    /// 用户选择的原启动器 EXE 路径；关闭联动后保留选择，文件恢复由安装器负责。
    pub launcher_paths: Vec<String>,
    /// 是否迁移存档（L1 可关断项，默认开启）。
    pub include_saves: bool,
    /// 是否迁移资源/光影包（L2 可关断项，默认开启）。
    pub include_packs: bool,
    /// 是否迁移模组数据（L3 可关断项，默认开启）。
    pub include_moddata: bool,
    /// 是否执行 options 智能合并（L4 可关断项，默认开启）。
    pub include_options: bool,
    /// 用户自定义迁移规则；None 表示未自定义（回退内置默认）。
    #[serde(default)]
    pub rules: Option<Vec<UserRuleEntry>>,
}

// ==================== 函数和方法定义 ====================

impl Default for AppConfig {
    /**
     * 函数职责：提供安全缺省配置。
     * 输入说明：无。
     * 输出说明：全部迁移项开启、自动备份开启、路径为空、规则未自定义。
     * 实现思路：逐字段赋默认值。
     */
    fn default() -> Self {
        Self {
            versions_dir: String::new(),
            last_source: String::new(),
            last_target: String::new(),
            auto_backup: true,
            follow_launchers: false,
            close_to_tray: false,
            launcher_paths: Vec::new(),
            include_saves: true,
            include_packs: true,
            include_moddata: true,
            include_options: true,
            rules: None,
        }
    }
}

/**
 * 函数职责：构造默认规则条目（内置路径常量 → 全部启用的用户规则）。
 * 输入说明：无。
 * 输出说明：按 L1→L2→L3→L4 排列的默认规则列表。
 * 实现思路：四级默认路径常量逐条映射，是"默认初始化值"的唯一出口。
 */
pub fn default_rule_entries() -> Vec<UserRuleEntry> {
    let mut entries = Vec::new();
    for path in L1_ASSETS {
        entries.push(UserRuleEntry {
            relative_path: (*path).to_string(),
            level: AssetLevel::Direct,
            enabled: true,
        });
    }
    for path in L2_ASSETS {
        entries.push(UserRuleEntry {
            relative_path: (*path).to_string(),
            level: AssetLevel::Incremental,
            enabled: true,
        });
    }
    for path in L3_ASSETS {
        entries.push(UserRuleEntry {
            relative_path: (*path).to_string(),
            level: AssetLevel::ModData,
            enabled: true,
        });
    }
    for path in L4_ASSETS {
        entries.push(UserRuleEntry {
            relative_path: (*path).to_string(),
            level: AssetLevel::SmartMerge,
            enabled: true,
        });
    }
    entries
}

impl AppConfig {
    /**
     * 函数职责：返回当前生效的用户规则条目（未自定义时回退内置默认）。
     * 输入说明：无。
     * 输出说明：规则条目列表（含禁用项，由调用方决定是否过滤）。
     * 实现思路：rules 为 None 时返回 default_rule_entries()，否则克隆存储值。
     */
    pub fn rule_entries(&self) -> Vec<UserRuleEntry> {
        self.rules.clone().unwrap_or_else(default_rule_entries)
    }

    /**
     * 函数职责：构造生成计划所需的生效规则注册表。
     * 输入说明：无。
     * 输出说明：仅含启用规则的注册表，按 L1→L2→L3→L4 排列保证执行顺序稳定。
     * 实现思路：取规则条目 → 过滤禁用项 → 按级别分组映射为 AssetRule
     *           （说明文案复用级别默认值）。
     */
    pub fn effective_registry(&self) -> RuleRegistry {
        let entries = self.rule_entries();
        let mut rules = Vec::new();
        for level in [
            AssetLevel::Direct,
            AssetLevel::Incremental,
            AssetLevel::ModData,
            AssetLevel::SmartMerge,
        ] {
            for entry in entries.iter().filter(|e| e.level == level && e.enabled) {
                rules.push(AssetRule {
                    relative_path: entry.relative_path.clone(),
                    level,
                    description: level_default_description(level).to_string(),
                });
            }
        }
        RuleRegistry { entries: rules }
    }

    /**
     * 函数职责：将持久化的布尔开关映射为迁移选项。
     * 输入说明：无。
     * 输出说明：与配置字段一一对应的 MigrationOptions。
     * 实现思路：逐字段拷贝。
     */
    pub fn migration_options(&self) -> MigrationOptions {
        MigrationOptions {
            auto_backup: self.auto_backup,
            include_saves: self.include_saves,
            include_packs: self.include_packs,
            include_moddata: self.include_moddata,
            include_options: self.include_options,
        }
    }

    /**
     * 函数职责：确定配置文件绝对路径。
     * 输入说明：无。
     * 输出说明：<用户配置目录>/packporter/config.json；无法定位用户目录时返回 None。
     * 实现思路：优先 PACKPORTER_CONFIG_DIR（测试隔离用），其次 AppData（Windows），
     *           回退 USER_PROFILE。
     */
    pub fn config_path() -> Option<PathBuf> {
        if let Ok(dir) = std::env::var("PACKPORTER_CONFIG_DIR") {
            return Some(PathBuf::from(dir).join(CONFIG_FILE_NAME));
        }
        let base = std::env::var("APPDATA").ok().or_else(|| std::env::var("USER_PROFILE").ok())?;
        Some(PathBuf::from(base).join("packporter").join(CONFIG_FILE_NAME))
    }

    /**
     * 函数职责：从磁盘加载配置；文件缺失或损坏时返回默认值。
     * 输入说明：无。
     * 输出说明：始终返回可用配置，不向调用方传播 IO 错误。
     * 实现思路：读文件 → serde_json 解析 → 失败时 Default。
     */
    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /**
     * 函数职责：将配置写入磁盘（覆盖保存，原子写）。
     * 输入说明：无。
     * 输出说明：成功返回 true；路径不可定位或写入失败返回 false（配置失败不阻断迁移）。
     * 实现思路：创建父目录 → 序列化 → 先写临时文件再重命名，避免写一半损坏配置。
     */
    pub fn save(&self) -> bool {
        let Some(path) = Self::config_path() else {
            return false;
        };
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return false;
            }
        }
        let Ok(text) = serde_json::to_string_pretty(self) else {
            return false;
        };
        // 原子写：临时文件写完再重命名覆盖。
        let temp = path.with_extension("json.tmp");
        if std::fs::write(&temp, text).is_err() {
            return false;
        }
        std::fs::rename(&temp, path).is_ok()
    }
}
