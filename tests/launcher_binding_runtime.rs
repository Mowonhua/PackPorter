//! 文件职责：通过原 EXE 路径验证受管 shim 联动和关闭设置后的文件还原。
//! 定义范围：真实 Windows 进程、隔离安装目录和公开安装/会话接口。

#![cfg(windows)]

use packporter::app_config::AppConfig;
use packporter::infra::{launcher_binding, launcher_companion};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// 测试只终止自己从标记文件取得的启动器 PID，并保留目录至句柄释放。
struct Fixture {
    directory: PathBuf,
    processes: Vec<u32>,
    windows: Vec<u32>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for &pid in &self.processes { terminate(pid); }
        for &pid in &self.windows { terminate(pid); }
        let config = AppConfig { follow_launchers: false, ..AppConfig::default() };
        config.save();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if launcher_companion::acquire_ui_instance().unwrap().is_some() { break; }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn terminate(pid: u32) {
    use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE}};
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if !handle.is_null() {
        unsafe { TerminateProcess(handle, 0); CloseHandle(handle); }
    }
}

fn alive(pid: u32) -> bool {
    use windows_sys::Win32::{Foundation::{CloseHandle, WAIT_TIMEOUT}, System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE}};
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() { return false; }
    let active = unsafe { WaitForSingleObject(handle, 0) } == WAIT_TIMEOUT;
    unsafe { CloseHandle(handle); }
    active
}

fn wait_for(message: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(Instant::now() < deadline, "{message}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn launch(original: &Path, marker: &Path, fixture: &mut Fixture) {
    use std::os::windows::process::CommandExt;
    let mut gateway = Command::new(original)
        .args(["--record", marker.to_str().unwrap(), "中文 空格", "a\"b", "--launcher", "--java", "--app", ""])
        .creation_flags(0x08000000)
        .spawn().unwrap();
    wait_for("受管入口没有启动原备份", || marker.is_file());
    let record = std::fs::read_to_string(marker).unwrap();
    let pid = record.lines().next().unwrap().parse().unwrap();
    fixture.processes.push(pid);
    assert!(record.contains("中文 空格") && record.contains("a\\\"b") && record.contains("--launcher") && record.contains("--java") && record.contains("--app") && record.contains("\"\""));
    wait_for("原路径 shim 未释放可执行文件", || gateway.try_wait().unwrap().is_some());
}

/// Cargo 只向当前包的集成测试提供 CARGO_BIN_EXE，跨包 shim 从同一构建目录定位。
/// workspace 全量测试会由 launcher 包的运行测试构建 shim；单跑本套件时须先构建它。
/// 显式路径供自定义构建目录使用，指定后不可静默回退到其他产物。
fn shim_executable() -> PathBuf {
    let path = std::env::var_os("PACKPORTER_SHIM_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let executable = std::env::current_exe().expect("无法定位测试程序");
            executable
                .parent()
                .and_then(Path::parent)
                .expect("测试程序应位于构建目录的 deps 子目录")
                .join("packporter-shim.exe")
        });
    assert!(
        path.is_file(),
        "未找到 shim：{}。请先运行 cargo build -p packporter-launcher --bin packporter-shim，并与测试使用相同的 profile/target；也可设置 PACKPORTER_SHIM_EXE",
        path.display()
    );
    path
}

#[test]
fn original_paths_launch_and_disabling_restores_bak_while_launchers_are_running() {
    assert!(launcher_companion::acquire_ui_instance().unwrap().is_some(), "请先关闭 PackPorter 再运行原路径联动测试");
    let directory = std::env::temp_dir().join(format!("packporter-binding-runtime-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    std::env::set_var("PACKPORTER_CONFIG_DIR", &directory);
    let mut fixture = Fixture { directory: directory.clone(), processes: Vec::new(), windows: Vec::new() };
    let source = directory.join("fixture.rs");
    std::fs::write(&source, r#"
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let marker = &args[2];
    std::fs::write(marker, format!("{}\n{:?}", std::process::id(), args)).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(90));
}
"#).unwrap();
    let first = directory.join("PCL 中文.exe");
    let second = directory.join("HMCL.exe");
    let output = Command::new("rustc").arg("--crate-name").arg("launcher_fixture").arg(&source).arg("-o").arg(&first).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    std::fs::copy(&first, &second).unwrap();
    let original = std::fs::read(&first).unwrap();
    let launchers = vec![first.clone(), second.clone()];
    let app = Path::new(env!("CARGO_BIN_EXE_packporter"));
    let shim = shim_executable();
    let mut config = AppConfig {
        follow_launchers: true,
        launcher_paths: launchers.iter().map(|path| path.to_string_lossy().into_owned()).collect(),
        ..AppConfig::default()
    };
    assert!(config.save());
    launcher_binding::apply_at(true, &launchers, app, &shim, &directory).unwrap();
    assert_eq!(std::fs::read(directory.join("PCL 中文.bak.exe")).unwrap(), original);
    launch(&first, &directory.join("first.txt"), &mut fixture);
    launch(&second, &directory.join("second.txt"), &mut fixture);
    wait_for("两个原入口未同时建立联动", || launcher_companion::launcher_count().unwrap_or(0) >= 2);
    wait_for("原入口未打开 PackPorter", || launcher_companion::acquire_ui_instance().unwrap().is_none());
    // 窗口可能在关闭配置后保持打开；仅记录本测试中央 shim 创建的窗口，供清理使用。
    let workers: Vec<u32> = std::fs::read_dir(directory.join("launcher-sessions")).unwrap()
        .filter_map(|entry| entry.ok()?.path().file_stem()?.to_str()?.parse().ok()).collect();
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    fixture.windows = system.processes().iter().filter_map(|(pid, process)| {
        (process.name() == "packporter.exe" && process.parent().is_some_and(|parent| workers.contains(&parent.as_u32())))
            .then_some(pid.as_u32())
    }).collect();
    assert!(!fixture.windows.is_empty(), "测试必须持有自己启动的窗口身份");
    // 复现设置保存次序：先写关闭状态，再还原入口；此时真实启动器仍在运行。
    config.follow_launchers = false;
    assert!(config.save());
    launcher_binding::apply_at(false, &launchers, app, &shim, &directory).unwrap();
    for path in &launchers { assert_eq!(std::fs::read(path).unwrap(), original); }
    assert!(!directory.join("PCL 中文.bak.exe").exists());
    assert!(!directory.join("HMCL.bak.exe").exists());
    assert!(fixture.processes.iter().all(|pid| alive(*pid)), "还原入口不得关闭原启动器");
    wait_for("关闭配置后中央 shim 仍驻留会话", || launcher_companion::launcher_count().unwrap_or(1) == 0);
}
