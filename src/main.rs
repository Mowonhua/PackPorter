// 文件职责：应用入口（桌面程序）：创建主窗口、装配交互回调并进入事件循环。

use packporter::app_config::AppConfig;
use packporter::app_controller::attach;
use packporter::services::folder_watcher::{FolderWatcherService, InstanceArrivalEvent};
use packporter::PackPorterWindow;
use slint::ComponentHandle;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

fn main() {
    // 加载持久化配置（缺失/损坏自动回退默认值），versions 路径供扫描与监控共用。
    let config = AppConfig::load();
    let watcher_root = PathBuf::from(&config.versions_dir);

    // 创建 UI 并把全部交互回调装配到服务层（扫描/计划/执行/打开备份目录）。
    let ui = PackPorterWindow::new().expect("UI 创建失败");
    let _handles = attach(&ui, watcher_root.clone());

    // 目录监控（模块 D）：对配置的 versions 根目录启动感知。
    if !watcher_root.as_os_str().is_empty() {
        spawn_watcher(&watcher_root, ui.as_weak());
    }

    ui.run().expect("UI 事件循环异常退出");
}

/**
 * 函数职责：启动 versions 目录监控线程，新实例就绪事件回写 UI 日志区。
 * 输入说明：root 为 versions 目录；weak 为 UI 弱引用（跨线程只读回写）。
 * 输出说明：无副作用返回；监控失败仅记录，不阻断 UI。
 * 实现思路：FolderWatcherService + 后台线程非阻塞接收事件，
 *           收到事件后通过 slint::invoke_from_event_loop 回主线程更新。
 */
fn spawn_watcher(root: &Path, weak: slint::Weak<PackPorterWindow>) {
    let probe = Arc::new(packporter::infra::watcher::SnapshotProbe);
    let (mut watcher, rx) = FolderWatcherService::new(root.to_path_buf(), probe);
    if watcher.start().is_err() {
        return;
    }
    // 事件接收线程：跨线程回写 UI 必须经由 invoke_from_event_loop。
    std::thread::spawn(move || {
        let receiver: Receiver<InstanceArrivalEvent> = rx;
        while let Ok(event) = receiver.recv() {
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    append_log(&ui, &format!("检测到新实例目录就绪：{}", event.dir_name));
                    ui.set_lock_warning_visible(false);
                }
            });
        }
        let _ = &mut watcher;
    });
}

/**
 * 函数职责：向 UI 日志区追加一行日志（监控线程复用）。
 * 输入说明：ui 为窗口引用；message 为日志文本。
 * 输出说明：副作用为更新 log-text 属性。
 * 实现思路：读取现有日志，追加换行与新内容后写回。
 */
fn append_log(ui: &PackPorterWindow, message: &str) {
    let existing = ui.get_log_text().to_string();
    ui.set_log_text(std::format!("{}{}\n", existing, message).into());
}
