//! 文件职责：共享配置文件位置，并读取启动器所需的最小配置。
//! 定义范围：配置路径解析与只读跟随开关；不写配置、不解析迁移规则。

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// GUI 与 shim 使用同一个文件名，以共享跟随开关和会话目录的基准位置。
pub const CONFIG_FILE_NAME: &str = "config.json";

/// 结构职责：从 GUI 配置中投影启动器所需的跟随开关。
/// 字段说明：follow_launchers 缺省关闭；其余字段由 GUI 自行解释。
/// 约束条件：不因无关迁移字段的格式演进而改变联动；无效开关不得启用联动。
#[derive(Default, Deserialize)]
#[serde(default)]
struct LauncherSettings {
    follow_launchers: bool,
}

/// 函数职责：定位 GUI 与 shim 共用的配置文件。
/// 输入说明：依次读取 PACKPORTER_CONFIG_DIR、APPDATA、USER_PROFILE。
/// 输出说明：返回配置文件路径；缺少所有可用环境变量时返回 None。
/// 实现思路：显式目录直接拼接文件名，用户目录先拼接 packporter 子目录。
pub fn config_path() -> Option<PathBuf> {
    if let Ok(directory) = std::env::var("PACKPORTER_CONFIG_DIR") {
        return Some(PathBuf::from(directory).join(CONFIG_FILE_NAME));
    }
    let base = std::env::var("APPDATA")
        .ok()
        .or_else(|| std::env::var("USER_PROFILE").ok())?;
    Some(
        PathBuf::from(base)
            .join("packporter")
            .join(CONFIG_FILE_NAME),
    )
}

/// 函数职责：读取指定配置文件中的启动器跟随开关。
/// 输入说明：path 为共享配置文件路径；仅 follow_launchers 属于此读取接口。
/// 输出说明：文件不可读、JSON 无效、开关缺失或类型错误时返回 false。
/// 实现思路：反序列化最小配置投影，忽略其余字段，使启动器不依赖迁移配置结构。
pub fn follow_launchers_at(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<LauncherSettings>(&raw).ok())
        .unwrap_or_default()
        .follow_launchers
}

/// 函数职责：取得当前用户的跟随开关，不缓存结果。
/// 输入说明：使用 config_path 定位文件。
/// 输出说明：无法定位文件时返回 false；其他情况遵循 follow_launchers_at。
/// 实现思路：每次调用都重读磁盘，使运行中的 shim 能观察关闭联动的设置。
pub fn follow_launchers() -> bool {
    config_path().is_some_and(|path| follow_launchers_at(&path))
}
