//! 文件职责：应用配置持久化模型：用户偏好（versions 路径、最近选择、迁移选项）。
//! 定义范围：AppConfig 结构、默认值与加载/保存实现。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ==================== 常量、枚举和类型别名 ====================

/// 配置文件名，存放于用户配置目录的 packporter 子目录下。
pub const CONFIG_FILE_NAME: &str = "config.json";

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：跨会话持久的用户配置。
 * 字段说明：全部字段可缺省为默认值；反序列化容忍未知字段（向前兼容）。
 * 约束条件：versions_dir 为空表示尚未选择；布尔开关缺省均为安全值（见 default()）。
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
    /// 是否迁移存档（L1 可关断项，默认开启）。
    pub include_saves: bool,
    /// 是否迁移资源/光影包（L2 可关断项，默认开启）。
    pub include_packs: bool,
    /// 是否迁移地图与辅助模组数据（L3 可关断项，默认开启）。
    pub include_moddata: bool,
    /// 是否执行 options 智能合并（L4 可关断项，默认开启）。
    pub include_options: bool,
}

// ==================== 函数和方法定义 ====================

impl Default for AppConfig {
    /**
     * 函数职责：提供安全缺省配置。
     * 输入说明：无。
     * 输出说明：全部迁移项开启、自动备份开启、路径为空。
     * 实现思路：逐字段赋默认值。
     */
    fn default() -> Self {
        Self {
            versions_dir: String::new(),
            last_source: String::new(),
            last_target: String::new(),
            auto_backup: true,
            include_saves: true,
            include_packs: true,
            include_moddata: true,
            include_options: true,
        }
    }
}

impl AppConfig {
    /**
     * 函数职责：确定配置文件绝对路径。
     * 输入说明：无。
     * 输出说明：<用户配置目录>/packporter/config.json；无法定位用户目录时返回 None。
     * 实现思路：优先 AppData（Windows），回退 USER_PROFILE。
     */
    pub fn config_path() -> Option<PathBuf> {
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
