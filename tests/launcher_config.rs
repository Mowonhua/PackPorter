//! 文件职责：验证旧配置兼容与启动器跟随偏好的序列化契约。
//! 定义范围：AppConfig 的公开 JSON 读写行为。

use packporter::app_config::AppConfig;

#[test]
fn older_config_does_not_enable_launcher_following() {
    let config: AppConfig = serde_json::from_str(r#"{"auto_backup":false}"#).unwrap();
    assert!(!config.follow_launchers);
    assert!(!config.auto_backup);
}

#[test]
fn enabled_launcher_preference_survives_json_roundtrip() {
    let config: AppConfig = serde_json::from_str(r#"{"follow_launchers":true}"#).unwrap();
    assert!(config.follow_launchers);
    let restored: AppConfig = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
    assert!(restored.follow_launchers);
}
