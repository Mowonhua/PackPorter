//! 文件职责：验证独立 shim 能启动使用 .NET Framework 路径初始化的启动器。
//! 定义范围：真实 CLR 子进程；隔离配置关闭联动，以只验证启动参数与路径边界。
#![cfg(windows)]

use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn framework_launcher_receives_a_conventional_application_path() {
    use std::os::windows::process::CommandExt;
    let directory =
        std::env::temp_dir().join(format!("packporter-framework-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("Program.cs");
    let executable = directory.join("Framework Launcher.bak.exe");
    let marker = directory.join("result.txt");
    std::fs::write(&source, r#"
using System;
using System.IO;
using System.Security.Permissions;
class Program {
    static void Main(string[] args) {
        try {
            new Uri("https://example.invalid");
            new FileIOPermission(FileIOPermissionAccess.Read, AppDomain.CurrentDomain.BaseDirectory);
            File.WriteAllText(args[0], "ok");
        } catch (Exception error) {
            File.WriteAllText(args[0], error.GetType().Name + ": " + error.Message);
        }
    }
}
"#).unwrap();
    let compiler = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap())
        .join("Microsoft.NET/Framework64/v4.0.30319/csc.exe");
    let build = Command::new(compiler)
        .creation_flags(0x08000000)
        .args(["/nologo", "/target:exe"])
        .arg(format!("/out:{}", executable.display()))
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stdout)
    );
    let status = Command::new(env!("CARGO_BIN_EXE_packporter-shim"))
        .creation_flags(0x08000000)
        .env("PACKPORTER_CONFIG_DIR", &directory)
        .arg("--launcher")
        .arg(&executable)
        .arg("--")
        .arg(&marker)
        .status()
        .unwrap();
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(15);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let result = std::fs::read_to_string(&marker).expect(".NET 启动器没有完成初始化");
    // 子进程已完成写入，允许 CLR 收尾释放测试 EXE 后再清理。
    std::thread::sleep(Duration::from_millis(200));
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(result, "ok", "shim 不应把内部规范化路径暴露给 CLR 初始化");
}
