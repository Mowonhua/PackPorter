//! 文件职责：将 GUI 设置应用到共享启动器绑定事务。
//! 定义范围：配置目录和应用位置适配，以及共享绑定接口的兼容导出。

use std::path::PathBuf;

pub use packporter_launcher::binding::{apply_at, read_binding, Binding};

/// 函数职责：将设置中的入口集合应用到当前用户的受管启动器。
/// 输入说明：关闭时忽略 launchers 并恢复全部清单条目。
/// 输出说明：失败时返回包含补偿失败信息的中文错误。
/// 实现思路：定位主程序、独立 shim 和清单目录后交给文件事务入口。
pub fn apply(enabled: bool, launchers: &[String]) -> Result<(), String> {
    let app = std::env::current_exe().map_err(|e| e.to_string())?;
    let shim = app.with_file_name("packporter-shim.exe");
    let config = crate::app_config::AppConfig::config_path().ok_or("无法定位用户配置目录")?;
    apply_at(
        enabled,
        &launchers.iter().map(PathBuf::from).collect::<Vec<_>>(),
        &app,
        &shim,
        config.parent().ok_or("配置路径缺少父目录")?,
    )
}
