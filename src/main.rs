// 文件职责：应用入口（桌面程序）：初始化 Slint UI、加载配置、装配服务层。

// 引入 Slint 生成的 UI 绑定（由 build.rs 编译 ui/packporter.slint 生成）。
slint::include_modules!();

use packporter::app_config::AppConfig;
use packporter::domain::error::PackError;
use packporter::services::folder_watcher::{FolderWatcherService, InstanceArrivalEvent};
use packporter::services::migration_service::MigrationService;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

fn main() {
    // 加载持久化配置（缺失/损坏自动回退默认值）。
    let config = AppConfig::load();

    // 构造 UI。
    let ui = PackPorterWindow::new().expect("UI 创建失败");
    let weak = ui.as_weak();

    // 服务层装配：versions 根目录来自配置（可能为空，扫描时校验）。
    let versions_root = Arc::new(Mutex::new(PathBuf::from(&config.versions_dir)));
    let migration = Arc::new(Mutex::new(Option::<Arc<Mutex<MigrationService>>>::None));

    // 配置回填 UI 初始状态。
    ui.set_lock_warning_visible(false);

    // 扫描请求：用当前 versions 根目录重建服务并填充实例列表。
    let weak_scan = weak.clone();
    let root_slot = versions_root.clone();
    let migration_slot = migration.clone();
    ui.on_scan_requested(move || {
        let ui = weak_scan.unwrap();
        let root = root_slot.lock().unwrap().clone();
        if root.as_os_str().is_empty() {
            append_log(&ui, "未配置 versions 目录，请先在配置中选择。");
            return;
        }
        let service = MigrationService::new(root);
        match service.instances.scan_instances() {
            Ok(profiles) => {
                let names: Vec<slint::SharedString> = profiles
                    .iter()
                    .map(|p| slint::SharedString::from(p.version.dir_name.as_str()))
                    .collect();
                ui.set_instance_names(slint::ModelRc::from(std::rc::Rc::new(
                    slint::VecModel::from(names),
                )));
                append_log(&ui, &format!("扫描完成，发现 {} 个实例。", profiles.len()));
                // 缓存服务实例供后续计划/执行复用。
                *migration_slot.lock().unwrap() = Some(Arc::new(Mutex::new(service)));
            }
            Err(e) => append_log(&ui, &format!("扫描失败：{e}")),
        }
    });

    // 迁移执行请求：确认后进入事务执行，进度回写 UI。
    let weak_exec = weak.clone();
    let migration_exec = migration.clone();
    ui.on_execute_requested(move || {
        let ui = weak_exec.unwrap();
        let Some(service_mutex) = migration_exec.lock().unwrap().clone() else {
            append_log(&ui, "请先扫描实例。");
            return;
        };
        let service = service_mutex.lock().unwrap();
        // 演示装配：扫描阶段缓存最近一次计划；此处直接以配置路径生成计划。
        // 计划生成与执行的完整参数绑定在 UI 状态接线完成后替换。
        append_log(&ui, "迁移执行流程已装配（计划生成待 UI 状态接线）。");
        let _ = &service;
    });

    // 打开备份目录请求：定位目标实例 backups 目录。
    let weak_open = weak.clone();
    ui.on_open_backup_folder(move || {
        let ui = weak_open.unwrap();
        append_log(&ui, "备份目录定位待 UI 状态接线。");
        let _ = ui;
    });

    // 目录监控（模块 D）：对配置的 versions 根目录启动感知。
    let watcher_root = versions_root.lock().unwrap().clone();
    if !watcher_root.as_os_str().is_empty() {
        spawn_watcher(&watcher_root, weak.clone());
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
fn spawn_watcher(root: &std::path::Path, weak: slint::Weak<PackPorterWindow>) {
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
 * 函数职责：向 UI 日志区追加一行日志。
 * 输入说明：ui 为窗口引用；message 为日志文本。
 * 输出说明：副作用为更新 log-text 属性。
 * 实现思路：读取现有日志，追加换行与新内容后写回。
 */
fn append_log(ui: &PackPorterWindow, message: &str) {
    let existing = ui.get_log_text().to_string();
    ui.set_log_text(std::format!("{}{}\n", existing, message).into());
}

// 未确认错误统一转日志文案的辅助（供后续 UI 接线复用）。
#[allow(dead_code)]
fn describe_error(err: &PackError) -> String {
    format!("{err}")
}
