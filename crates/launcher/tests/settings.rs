//! 文件职责：验证独立启动器读取共享配置时的默认值和字段边界。
//! 定义范围：通过公开读取接口驱动真实临时配置文件。

use packporter_launcher::settings::follow_launchers_at;

#[test]
fn reads_only_follow_flag_and_defaults_to_disabled_on_invalid_input() {
    let directory = std::env::temp_dir().join(format!(
        "packporter-launcher-settings-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("config.json");
    for (raw, expected) in [
        (r#"{"follow_launchers":true}"#, true),
        (r#"{"follow_launchers":false}"#, false),
        (r#"{}"#, false),
        (r#"{"follow_launchers":"true"}"#, false),
        (r#"{"follow_launchers":null}"#, false),
        (r#"{"follow_launchers":true"#, false),
        (
            r#"{"follow_launchers":true,"rules":"unknown format","future_setting":{}}"#,
            true,
        ),
    ] {
        std::fs::write(&path, raw).unwrap();
        assert_eq!(follow_launchers_at(&path), expected, "{raw}");
    }
    std::fs::remove_file(&path).unwrap();
    assert!(!follow_launchers_at(&path));
    assert!(!follow_launchers_at(&directory));
    std::fs::remove_dir(&directory).unwrap();
}
