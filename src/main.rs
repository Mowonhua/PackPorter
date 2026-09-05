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

    // 无边框窗口镶边（Windows）：原生窗口在事件循环启动并显示后才存在，
    // 先调度安装，句柄未就绪则以短间隔重试。
    install_frameless_chrome(ui.as_weak());

    ui.run().expect("UI 事件循环异常退出");
}

/**
 * 函数职责：安装无边框窗口镶边（Windows）：命中测试子类化提供标题栏拖动、
 *           八向边缘缩放与双击最大化，并开启 DWM 圆角与投影。
 * 输入说明：weak 为主窗口弱引用。
 * 输出说明：无返回值；窗口已销毁则静默放弃；句柄持续未就绪（重试耗尽）仅告警，
 *           窗口仍可正常显示与操作，只是失去拖动/缩放。
 * 实现思路：取 HWND（raw-window-handle）→ 注入几何探针（读取 UI 标题栏属性）→
 *           infra 层安装子类；创建未完成时经事件循环定时器重试。
 */
#[cfg(windows)]
fn install_frameless_chrome(weak: slint::Weak<PackPorterWindow>) {
    const MAX_TRIES: u32 = 20;

    fn hwnd_of(ui: &PackPorterWindow) -> Option<packporter::infra::window_chrome::NativeWindow> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let slint_handle = ui.window().window_handle();
        let handle = slint_handle.window_handle().ok()?;
        match handle.as_raw() {
            RawWindowHandle::Win32(win32) => {
                Some(win32.hwnd.get() as packporter::infra::window_chrome::NativeWindow)
            }
            _ => None,
        }
    }

    // 几何探针：命中测试时读取 UI 当前布局；窗口销毁后返回全零（无可拖动区）。
    fn probe_of(ui: &PackPorterWindow) -> packporter::infra::window_chrome::GeometryProbe {
        use packporter::infra::window_chrome::TitlebarGeometry;
        let weak = ui.as_weak();
        Box::new(move || {
            let Some(ui) = weak.upgrade() else { return TitlebarGeometry::default() };
            TitlebarGeometry {
                caption_height: ui.get_titlebar_height(),
                controls_x: ui.get_titlebar_controls_x(),
                inline_client: (ui.get_titlebar_gear_start(), ui.get_titlebar_gear_end()),
            }
        })
    }

    fn attempt(weak: slint::Weak<PackPorterWindow>, tries_left: u32) {
        let Some(ui) = weak.upgrade() else { return };
        match hwnd_of(&ui) {
            Some(hwnd) => unsafe {
                packporter::infra::window_chrome::install(hwnd, probe_of(&ui));
            },
            None if tries_left > 0 => slint::Timer::single_shot(
                std::time::Duration::from_millis(50),
                move || attempt(weak, tries_left - 1),
            ),
            None => eprintln!("警告：未获取到原生窗口句柄，窗口拖动/缩放不可用"),
        }
    }

    attempt(weak, MAX_TRIES);
}

/// 非 Windows 平台无原生镶边实现，保持入口装配一致。
#[cfg(not(windows))]
fn install_frameless_chrome(_weak: slint::Weak<PackPorterWindow>) {}

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
