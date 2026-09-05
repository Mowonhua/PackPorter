// 文件职责：应用入口（桌面程序）：创建主窗口、装配交互回调并进入事件循环。

use packporter::app_controller::{attach, WatcherRestartHook};
use packporter::infra::watcher::SnapshotProbe;
use packporter::services::folder_watcher::{FolderWatcherService, InstanceArrivalEvent};
use packporter::PackPorterWindow;
use slint::ComponentHandle;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

fn main() {
    // 加载持久化配置（缺失/损坏自动回退默认值），versions 路径供扫描与监控共用。
    let config = packporter::app_config::AppConfig::load();
    let watcher_root = PathBuf::from(&config.versions_dir);

    // 创建 UI 并把全部交互回调装配到服务层（扫描/计划/执行/设置）。
    let ui = PackPorterWindow::new().expect("UI 创建失败");
    let watcher_state: Arc<Mutex<Option<ActiveWatcher>>> = Arc::new(Mutex::new(None));
    let hook = make_restart_hook(ui.as_weak(), watcher_state.clone());
    let _handles = attach(&ui, watcher_root.clone(), hook);

    // 目录监控（模块 D）：对配置的 versions 根目录启动感知；设置页改目录时经钩子重启。
    // notify 的 watch 在 Windows 上会同步等待内部线程应答（无超时 recv），
    // 因此启动/重启一律放后台线程，避免阻塞事件循环。
    if !watcher_root.as_os_str().is_empty() {
        let state = watcher_state.clone();
        let weak = ui.as_weak();
        std::thread::spawn(move || start_watcher(&watcher_root, weak, &state));
    }

    ui.run().expect("UI 事件循环异常退出");
}

/**
 * 结构职责：一份活跃的目录监控会话：服务 + 启动句柄。
 * 字段说明：服务被替换（drop）时事件通道关闭，旧接收线程自然退出。
 * 约束条件：同一时刻至多一份活跃会话（由 watcher_state 的 Option 语义保证）。
 */
struct ActiveWatcher {
    service: FolderWatcherService,
    handle: u64,
}

/**
 * 函数职责：构造 versions 目录变更钩子：停止旧会话并以新目录重启监控。
 * 输入说明：weak 为 UI 弱引用；state 为活跃会话槽。
 * 输出说明：可在事件循环线程安全调用的重启钩子。
 * 实现思路：整个停止/启动流程放入后台线程（理由见 main 内注释）；
 *           先 stop 旧会话释放资源（stop_flag 使轮询线程退出、drop 服务使
 *           旧接收线程 recv 失败退出），再以新目录启动新会话。
 */
fn make_restart_hook(
    weak: slint::Weak<PackPorterWindow>,
    state: Arc<Mutex<Option<ActiveWatcher>>>,
) -> WatcherRestartHook {
    Arc::new(move |root: &str| {
        let root = root.to_string();
        let weak = weak.clone();
        let state = state.clone();
        std::thread::spawn(move || {
            let mut guard = state.lock().unwrap();
            if let Some(mut active) = guard.take() {
                active.service.stop(active.handle);
            }
            start_watcher(Path::new(&root), weak, &state);
        });
    })
}

/**
 * 函数职责：对指定 versions 目录启动一轮监控并拉起事件接收线程。
 * 输入说明：root 为 versions 目录；weak 为 UI 弱引用；state 为活跃会话槽。
 * 输出说明：无副作用返回；监控启动失败仅放弃本轮（不阻断 UI）。
 * 实现思路：FolderWatcherService + 后台线程非阻塞接收事件，
 *           收到事件后通过 slint::invoke_from_event_loop 回主线程更新状态栏。
 */
fn start_watcher(
    root: &Path,
    weak: slint::Weak<PackPorterWindow>,
    state: &Mutex<Option<ActiveWatcher>>,
) {
    let probe = Arc::new(SnapshotProbe);
    let (mut watcher, rx) = FolderWatcherService::new(root.to_path_buf(), probe);
    let Ok(handle) = watcher.start() else {
        return;
    };
    *state.lock().unwrap() = Some(ActiveWatcher { service: watcher, handle });
    // 事件接收线程：跨线程回写 UI 必须经由 invoke_from_event_loop。
    std::thread::spawn(move || {
        let receiver: Receiver<InstanceArrivalEvent> = rx;
        while let Ok(event) = receiver.recv() {
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_status_kind("info".into());
                    ui.set_status_text(
                        format!("检测到新实例「{}」就绪，点击「重新扫描」加入列表。", event.dir_name)
                            .into(),
                    );
                    ui.set_lock_warning_visible(false);
                }
            });
        }
    });
}
