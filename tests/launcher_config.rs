//! 文件职责：验证旧配置兼容与启动器跟随偏好的序列化契约。
//! 定义范围：AppConfig 的公开 JSON 读写行为。

use packporter::app_config::AppConfig;

#[test]
fn older_config_does_not_enable_launcher_following() {
    let config: AppConfig = serde_json::from_str(r#"{"auto_backup":false}"#).unwrap();
    assert!(!config.follow_launchers);
    assert!(!config.close_to_tray);
    assert!(!config.auto_backup);
}

#[test]
fn close_to_tray_preference_survives_json_roundtrip() {
    let config: AppConfig = serde_json::from_str(r#"{"close_to_tray":true}"#).unwrap();
    assert!(config.close_to_tray);
    let restored: AppConfig = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert!(restored.close_to_tray);
}

#[test]
fn enabled_launcher_preference_survives_json_roundtrip() {
    let config: AppConfig = serde_json::from_str(r#"{"follow_launchers":true}"#).unwrap();
    assert!(config.follow_launchers);
    let restored: AppConfig = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert!(restored.follow_launchers);
}

#[test]
fn shim_reads_follow_preference_saved_by_gui() {
    use std::{ffi::OsString, path::PathBuf};

    struct ConfigDirectory {
        directory: PathBuf,
        previous: Option<OsString>,
    }

    impl Drop for ConfigDirectory {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var("PACKPORTER_CONFIG_DIR", value),
                None => std::env::remove_var("PACKPORTER_CONFIG_DIR"),
            }
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    let directory = std::env::temp_dir().join(format!(
        "packporter-launcher-config-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let fixture = ConfigDirectory {
        directory,
        previous: std::env::var_os("PACKPORTER_CONFIG_DIR"),
    };
    // 本测试程序其他用例仅操作 JSON，不访问进程环境；隔离目录避免触及用户配置。
    std::env::set_var("PACKPORTER_CONFIG_DIR", &fixture.directory);
    let path = AppConfig::config_path().unwrap();
    assert_eq!(packporter_launcher::settings::config_path(), Some(path.clone()));
    let mut config = AppConfig {
        follow_launchers: true,
        versions_dir: "E:\\中文 整合包\\versions".into(),
        rules: Some(packporter::app_config::default_rule_entries()),
        ..AppConfig::default()
    };
    assert!(config.save());
    assert!(packporter_launcher::settings::follow_launchers_at(&path));
    config.follow_launchers = false;
    assert!(config.save());
    assert!(!packporter_launcher::settings::follow_launchers_at(&path));
}
