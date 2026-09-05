//! 文件职责：UI 交互控制器：将主窗口回调装配到服务层，供桌面入口与集成测试复用。
//! 定义范围：共享上下文槽、attach 装配函数、扫描/计划/执行的编排与状态栏反馈辅助。
//! 反馈约定：用户提示统一走单行状态栏（覆盖式），不再使用追加式日志；
//!           扫描与计划生成均在后台线程执行，UI 只在事件循环线程写属性。

use crate::app_config::AppConfig;
use crate::domain::error::{PackError, PackResult};
use crate::domain::instance::{
    AssetLevel, DecisionAction, InstanceProfile, MigrationPlan, MigrationProgress,
};
use crate::services::migration_service::MigrationService;
use crate::{PlanEntryView, PackPorterWindow};
use slint::ComponentHandle;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// 扫描后缓存的实例画像列表（与下拉框模型同源同序）。
type ProfilesSlot = Arc<Mutex<Vec<InstanceProfile>>>;
// 扫描后缓存的迁移编排器，供计划与执行复用。
type ServiceSlot = Arc<Mutex<Option<Arc<Mutex<MigrationService>>>>>;
// 最近一次生成的迁移计划，供执行与打开备份目录复用。
type PlanSlot = Arc<Mutex<Option<MigrationPlan>>>;

// versions 目录变更时重启目录监控的钩子（由入口层注入，测试可传空实现）。
pub type WatcherRestartHook = Arc<dyn Fn(&str) + Send + Sync>;

/**
 * 结构职责：attach 装配的共享上下文：全部回调共用的状态槽与标志位。
 * 字段说明：槽内容仅由回调在事件循环线程更新；busy/executing/pending 为原子标志。
 * 约束条件：外部不得手工写入业务状态；UI 属性写入只允许发生在事件循环线程。
 */
struct ControllerCtx {
    /// 当前 versions 根目录（设置保存时可变更）。
    root_dir: Arc<Mutex<PathBuf>>,
    /// 持久化配置（含迁移开关与最近选择）。
    config: Arc<Mutex<AppConfig>>,
    /// 扫描/计划/执行互斥忙碌标志：同一时刻只允许一个后台编排任务。
    busy: Arc<AtomicBool>,
    /// 迁移事务执行中标志（独立于 busy，用于结果呈现与进度卡片）。
    executing: Arc<AtomicBool>,
    /// 忙碌期间收到的新计划请求标记：任务收尾后自动补一次计划。
    replan_pending: Arc<AtomicBool>,
    /// 扫描出的实例画像（与下拉框模型同序）。
    profiles: ProfilesSlot,
    /// 当前迁移编排器（扫描成功后可用）。
    service: ServiceSlot,
    /// 最近一次生成的迁移计划。
    plan: PlanSlot,
    /// versions 目录变更钩子。
    watcher_hook: WatcherRestartHook,
}

/**
 * 结构职责：attach 装配完成后暴露给调用方的共享状态槽。
 * 字段说明：测试经这些槽断言回调副作用；入口层通常不读取。
 * 约束条件：槽内容仅由回调更新，外部不得手工写入业务状态。
 */
pub struct ControllerHandles {
    /// 扫描出的实例画像（与下拉框模型同序）。
    pub profiles: ProfilesSlot,
    /// 当前迁移编排器（扫描成功后可用）。
    pub service: ServiceSlot,
    /// 最近一次生成的迁移计划。
    pub plan: PlanSlot,
}

/**
 * 函数职责：将窗口全部交互回调装配到服务层（UI 可交互的核心接线点）。
 * 输入说明：ui 为主窗口引用；versions_root 为扫描用 versions 目录；
 *           watcher_hook 为 versions 目录变更时重启目录监控的回调。
 * 输出说明：返回共享状态槽供测试断言；无 panic 路径。
 * 实现思路：注册扫描（后台执行，结果回填下拉框并恢复上次选择）、
 *           计划（选择变化自动触发 + 忙碌期间请求合并为收尾补跑）、
 *           执行（后台事务，进度与结果经事件循环回写）、
 *           打开备份目录、设置页四回调（浏览/保存/取消，保存后按需重扫或重规划）；
 *           装配末尾在已配置目录时自动启动首次扫描。
 */
pub fn attach(
    ui: &PackPorterWindow,
    versions_root: PathBuf,
    watcher_hook: WatcherRestartHook,
) -> ControllerHandles {
    let config = Arc::new(Mutex::new(AppConfig::load()));
    // 未配置目录时给出指向设置页的引导，而不是提示改配置文件。
    ui.set_has_versions_dir(!versions_root.as_os_str().is_empty());
    if versions_root.as_os_str().is_empty() {
        set_status(ui, "info", "尚未配置 Minecraft 目录，请点击右上角「设置」完成配置。");
    }
    let ctx = Arc::new(ControllerCtx {
        root_dir: Arc::new(Mutex::new(versions_root)),
        config,
        busy: Arc::new(AtomicBool::new(false)),
        executing: Arc::new(AtomicBool::new(false)),
        replan_pending: Arc::new(AtomicBool::new(false)),
        profiles: Arc::new(Mutex::new(Vec::new())),
        service: Arc::new(Mutex::new(None)),
        plan: Arc::new(Mutex::new(None)),
        watcher_hook,
    });

    // ==================== 扫描回调 ====================
    let weak = ui.as_weak();
    let ctx_scan = ctx.clone();
    ui.on_scan_requested(move || { try_start_scan(&ctx_scan, &weak.unwrap()); });

    // ==================== 计划回调（自动 + 手动共用） ====================
    let weak = ui.as_weak();
    let ctx_plan = ctx.clone();
    ui.on_plan_requested(move || { try_start_plan(&ctx_plan, &weak.unwrap()); });

    // ==================== 执行回调 ====================
    let weak = ui.as_weak();
    let ctx_exec = ctx.clone();
    ui.on_execute_requested(move || {
        let ui = weak.unwrap();
        if ctx_exec.busy.load(Ordering::Relaxed) {
            return;
        }
        // 按钮可用性已由 plan-ready 门控；此处兜底校验。
        let Some(plan_snapshot) = ctx_exec.plan.lock().unwrap().clone() else {
            return;
        };
        ctx_exec.busy.store(true, Ordering::Relaxed);
        ctx_exec.executing.store(true, Ordering::Relaxed);
        ui.set_busy(true);
        ui.set_executing(true);
        ui.set_progress_percent(-1);
        ui.set_progress_text("正在备份与复制文件…".into());
        set_status(&ui, "info", "迁移执行中，请勿关闭游戏或修改实例目录。");

        let service_bg = ctx_exec.service.clone();
        let weak_bg = weak.clone();
        // 回调闭包可多次触发：进入后台线程前先克隆句柄，避免从 FnMut 中移出捕获变量。
        let ctx_bg = ctx_exec.clone();
        std::thread::spawn(move || {
            // 服务槽与弱引用均为 Clone 型句柄：显式克隆后移入后台线程，
            // 避免回调闭包（可多次触发）按引用捕获导致的借用逃逸。
            let result = match service_bg.lock().unwrap().clone() {
                Some(service) => {
                    let service = service.lock().unwrap();
                    service.execute_plan(&plan_snapshot, true, &mut |p: MigrationProgress| {
                        report_progress(&weak_bg, &p);
                    })
                }
                None => Err(PackError::InvalidPlan("尚未扫描实例".to_string())),
            };
            let busy = ctx_bg.busy.clone();
            let executing = ctx_bg.executing.clone();
            let _ = slint::invoke_from_event_loop(move || {
                busy.store(false, Ordering::Relaxed);
                executing.store(false, Ordering::Relaxed);
                if let Some(ui) = weak_bg.upgrade() {
                    ui.set_busy(false);
                    ui.set_executing(false);
                    match result {
                        Ok(outcome) => {
                            ui.set_progress_percent(100);
                            set_status(&ui, "success", &format!("迁移完成：{}", outcome.report));
                        }
                        // 事务失败已自动回滚：实例恢复至迁移前状态，需明确告知。
                        Err(PackError::RolledBack { reason, .. }) => {
                            ui.set_progress_percent(-1);
                            set_status(&ui, "error", &format!("迁移失败，已回滚：{reason}"));
                        }
                        Err(e) => {
                            ui.set_progress_percent(-1);
                            set_status(&ui, "error", &format!("迁移失败：{e}"));
                        }
                    }
                }
            });
        });
    });

    // ==================== 打开备份目录回调 ====================
    let weak = ui.as_weak();
    let ctx_open = ctx.clone();
    ui.on_open_backup_folder(move || {
        let ui = weak.unwrap();
        let Some(backup_dir) = ctx_open.plan.lock().unwrap().as_ref().map(|p| p.backup_dir.clone())
        else {
            return;
        };
        if !backup_dir.exists() {
            set_status(&ui, "info", "备份目录将在首次迁移后创建。");
            return;
        }
        // explorer 独立进程打开；成功不打扰用户，失败才提示。
        if std::process::Command::new("explorer").arg(&backup_dir).spawn().is_err() {
            set_status(&ui, "error", "打开备份目录失败。");
        }
    });

    // ==================== 设置页回调 ====================
    let weak = ui.as_weak();
    let ctx_settings = ctx.clone();
    ui.on_open_settings(move || {
        let ui = weak.unwrap();
        let config = ctx_settings.config.lock().unwrap();
        ui.set_settings_versions_dir(config.versions_dir.clone().into());
        ui.set_settings_auto_backup(config.auto_backup);
        ui.set_settings_include_saves(config.include_saves);
        ui.set_settings_include_packs(config.include_packs);
        ui.set_settings_include_moddata(config.include_moddata);
        ui.set_settings_include_options(config.include_options);
        ui.set_settings_open(true);
    });

    let weak = ui.as_weak();
    ui.on_cancel_settings(move || {
        weak.unwrap().set_settings_open(false);
    });

    let weak = ui.as_weak();
    ui.on_browse_versions_dir(move || {
        let ui = weak.unwrap();
        // 原生目录选择对话框：模态运行于事件循环之上，不另起线程。
        if let Some(dir) = rfd::FileDialog::new()
            .set_title("选择 .minecraft/versions 目录")
            .pick_folder()
        {
            ui.set_settings_versions_dir(dir.to_string_lossy().to_string().into());
        }
    });

    let weak = ui.as_weak();
    let ctx_save = ctx.clone();
    ui.on_save_settings(move || {
        let ui = weak.unwrap();
        let dir = ui.get_settings_versions_dir().trim().to_string();
        // 目录有效性：空路径或不存在都拒绝保存，留在设置页等待修正。
        if dir.is_empty() || !std::path::Path::new(&dir).is_dir() {
            set_status(&ui, "error", "目录不存在或不可访问，请检查路径后重试。");
            return;
        }
        let (dir_changed, toggles_changed) = {
            let mut config = ctx_save.config.lock().unwrap();
            let dir_changed = config.versions_dir != dir;
            let toggles_changed = config.auto_backup != ui.get_settings_auto_backup()
                || config.include_saves != ui.get_settings_include_saves()
                || config.include_packs != ui.get_settings_include_packs()
                || config.include_moddata != ui.get_settings_include_moddata()
                || config.include_options != ui.get_settings_include_options();
            config.versions_dir = dir.clone();
            config.auto_backup = ui.get_settings_auto_backup();
            config.include_saves = ui.get_settings_include_saves();
            config.include_packs = ui.get_settings_include_packs();
            config.include_moddata = ui.get_settings_include_moddata();
            config.include_options = ui.get_settings_include_options();
            config.save();
            (dir_changed, toggles_changed)
        };
        ui.set_settings_open(false);
        if dir_changed {
            // 目录变更：更新扫描根、重启目录监控并自动重扫（旧计划随之失效）。
            *ctx_save.root_dir.lock().unwrap() = PathBuf::from(&dir);
            (ctx_save.watcher_hook)(&dir);
            ui.set_has_versions_dir(true);
            set_status(&ui, "info", "设置已保存，正在重新扫描实例…");
            try_start_scan(&ctx_save, &ui);
        } else if toggles_changed {
            // 仅开关变化：保留当前选择，直接重出计划。
            set_status(&ui, "info", "设置已保存，正在按新选项重新生成计划…");
            try_start_plan(&ctx_save, &ui);
        } else {
            set_status(&ui, "success", "设置已保存。");
        }
    });

    // 启动即自动扫描：配置过目录时打开应用就有实例可选。
    if !ctx.root_dir.lock().unwrap().as_os_str().is_empty() {
        try_start_scan(&ctx, ui);
    }

    ControllerHandles { profiles: ctx.profiles.clone(), service: ctx.service.clone(), plan: ctx.plan.clone() }
}

// ==================== 编排辅助（UI 线程调用） ====================

/**
 * 函数职责：尝试启动后台扫描（回调入口与设置保存共用）。
 * 输入说明：ctx 为共享上下文；ui 为窗口引用。
 * 输出说明：true 表示已启动后台扫描；忙碌或目录未配置时返回 false。
 * 实现思路：busy 互斥 → 目录校验 → 后台线程扫描 → 事件循环回填模型并恢复上次选择。
 */
fn try_start_scan(ctx: &Arc<ControllerCtx>, ui: &PackPorterWindow) -> bool {
    if ctx.busy.load(Ordering::Relaxed) {
        return false;
    }
    let root = ctx.root_dir.lock().unwrap().clone();
    if root.as_os_str().is_empty() {
        set_status(ui, "error", "尚未配置 Minecraft 目录，请点击右上角「设置」完成配置。");
        return false;
    }
    ctx.busy.store(true, Ordering::Relaxed);
    ui.set_busy(true);
    set_status(ui, "info", "正在扫描实例…");

    let ctx_bg = CtxShare::new(ctx, ui.as_weak());
    std::thread::spawn(move || {
        // 每次扫描重建编排器，保证绑定最新的 versions 根目录。
        let service = Arc::new(Mutex::new(MigrationService::new(root)));
        let scan_result = service.lock().unwrap().instances.scan_instances();
        let _ = slint::invoke_from_event_loop(move || {
            ctx_bg.ctx.busy.store(false, Ordering::Relaxed);
            let Some(ui) = ctx_bg.weak.upgrade() else { return };
            ui.set_busy(false);
            match scan_result {
                Ok(found) => {
                    let count = found.len();
                    // 首行为占位行（索引 0 = 未选择）：ComboBox 会把越界的 -1 收敛为 0，
                    // 用占位行表达"未选择"可避免启动时误显示第一个实例。
                    let mut names: Vec<slint::SharedString> =
                        vec![slint::SharedString::from("— 未选择 —")];
                    names.extend(
                        found
                            .iter()
                            .map(|p| slint::SharedString::from(p.version.dir_name.as_str())),
                    );
                    ui.set_instance_names(slint::ModelRc::from(std::rc::Rc::new(
                        slint::VecModel::from(names),
                    )));
                    // 先落服务与画像，再动选中索引：changed 回调触发的计划请求能立即生效。
                    *ctx_bg.ctx.service.lock().unwrap() = Some(service);
                    *ctx_bg.ctx.profiles.lock().unwrap() = found.clone();
                    *ctx_bg.ctx.plan.lock().unwrap() = None;
                    ui.set_plan_ready(false);
                    ui.set_lock_warning_visible(false);
                    clear_plan(&ui);
                    // 新列表与旧选中索引必然失配，重置为未选择（占位行）。
                    ui.set_source_index(0);
                    ui.set_target_index(0);
                    // 恢复上次迁移选择（源与目标同名时只恢复源，避免启动即报错）。
                    let (last_source, last_target) = {
                        let config = ctx_bg.ctx.config.lock().unwrap();
                        (config.last_source.clone(), config.last_target.clone())
                    };
                    if let Some(pos) = found.iter().position(|p| p.version.dir_name == last_source) {
                        ui.set_source_index(pos as i32 + 1);
                    }
                    if last_target != last_source {
                        if let Some(pos) =
                            found.iter().position(|p| p.version.dir_name == last_target)
                        {
                            ui.set_target_index(pos as i32 + 1);
                        }
                    }
                    // 恢复流程不必然选满：未选满时给出下一步指引。
                    if count == 0 {
                        set_status(&ui, "info", "未发现任何实例，请确认目录是否为 .minecraft/versions。");
                    } else if ui.get_source_index() < 1 || ui.get_target_index() < 1 {
                        set_status(&ui, "info", &format!("已发现 {count} 个实例，请选择源与目标。"));
                    }
                }
                Err(e) => set_status(&ui, "error", &format!("扫描失败：{e}")),
            }
        });
    });
    true
}

/**
 * 函数职责：尝试启动后台计划生成（选择变化自动触发、手动重生成与设置保存共用）。
 * 输入说明：ctx 为共享上下文；ui 为窗口引用。
 * 输出说明：true 表示已启动后台计划；忙碌/执行中/索引无效时返回 false。
 * 实现思路：busy 互斥（忙碌期间标记 pending，收尾补跑）→ 索引与占用校验 →
 *           后台线程生成计划 → 事件循环渲染并持久化最近选择。
 */
fn try_start_plan(ctx: &Arc<ControllerCtx>, ui: &PackPorterWindow) -> bool {
    if ctx.busy.load(Ordering::Relaxed) {
        // 迁移执行期间不补跑计划；其余忙碌场景收尾后自动按最新选择重试。
        if !ctx.executing.load(Ordering::Relaxed) {
            ctx.replan_pending.store(true, Ordering::Relaxed);
        }
        return false;
    }
    let Some(service_mutex) = ctx.service.lock().unwrap().clone() else {
        // 未扫描时的自动触发是正常路径（索引复位等），静默忽略。
        return false;
    };
    let profiles = ctx.profiles.lock().unwrap();
    let (s, t) = (ui.get_source_index(), ui.get_target_index());
    // 索引有效性：0 为占位行（未选择）；其余按 偏移 1 映射到画像列表。
    if s < 1 || t < 1 || s as usize > profiles.len() || t as usize > profiles.len() {
        return false;
    }
    let source = profiles[s as usize - 1].clone();
    let target = profiles[t as usize - 1].clone();
    drop(profiles);
    if source.root_dir == target.root_dir {
        set_status(ui, "error", "源实例与目标实例不能相同。");
        return false;
    }
    let options = ctx.config.lock().unwrap().migration_options();

    ctx.busy.store(true, Ordering::Relaxed);
    ctx.replan_pending.store(false, Ordering::Relaxed);
    ui.set_busy(true);
    ui.set_plan_ready(false);
    ui.set_lock_warning_visible(false);
    clear_plan(ui);
    set_status(ui, "info", "正在生成迁移计划…");

    let ctx_bg = CtxShare::new(ctx, ui.as_weak());
    std::thread::spawn(move || {
        let service = service_mutex.lock().unwrap();
        // 占用检测：任一端被运行中的 java 进程占用即阻断计划生成。
        let lock_result: PackResult<()> = (|| {
            for profile in [&source, &target] {
                service.instances.ensure_unlocked(profile)?;
            }
            Ok(())
        })();
        let plan_result = lock_result.and_then(|()| service.plan_migration(&source, &target, options));
        drop(service);
        let _ = slint::invoke_from_event_loop(move || {
            ctx_bg.ctx.busy.store(false, Ordering::Relaxed);
            let Some(ui) = ctx_bg.weak.upgrade() else { return };
            ui.set_busy(false);
            match plan_result {
                Ok(p) => {
                    apply_plan(&ui, &p);
                    *ctx_bg.ctx.plan.lock().unwrap() = Some(p.clone());
                    // 记住本次选择，下次启动自动恢复。
                    let mut config = ctx_bg.ctx.config.lock().unwrap();
                    config.last_source = p.source.version.dir_name.clone();
                    config.last_target = p.target.version.dir_name.clone();
                    config.save();
                }
                Err(PackError::InstanceLocked { instance_name, pid, process_name }) => {
                    ui.set_lock_warning_visible(true);
                    set_status(
                        &ui,
                        "error",
                        &format!("{instance_name} 正被 {process_name}（PID {pid}）占用，请关闭游戏后点击「重新生成」。"),
                    );
                }
                Err(e) => set_status(&ui, "error", &format!("计划生成失败：{e}")),
            }
            // 忙碌期间有新的选择变化：按最新选择补跑一次计划。
            if ctx_bg.ctx.replan_pending.swap(false, Ordering::Relaxed) {
                try_start_plan(&ctx_bg.ctx, &ui);
            }
        });
    });
    true
}

// ==================== UI 呈现辅助（UI 线程调用） ====================

/**
 * 函数职责：ControllerCtx 的可克隆借出视图：弱引用 + 上下文的克隆句柄。
 * 字段说明：供后台线程 move 使用，收尾经事件循环写回。
 * 实现思路：全部字段为 Clone 型句柄，统一一次性克隆。
 */
struct CtxShare {
    weak: slint::Weak<PackPorterWindow>,
    ctx: Arc<ControllerCtx>,
}

impl CtxShare {
    fn new(ctx: &Arc<ControllerCtx>, weak: slint::Weak<PackPorterWindow>) -> Self {
        Self { ctx: ctx.clone(), weak }
    }
}

/**
 * 函数职责：将生成的迁移计划渲染到预览区（摘要 + 逐规则明细行）。
 * 输入说明：ui 为窗口引用；plan 为 plan_migration 产物。
 * 输出说明：副作用为更新 plan-summary、plan-entries、plan-ready 并清除结果卡片。
 * 实现思路：逐条目统计复制/保留数量，L4 条目（无文件决策）按合并键数生成动作标签；
 *           摘要中列出被设置关闭而跳过的级别。
 */
fn apply_plan(ui: &PackPorterWindow, plan: &MigrationPlan) {
    let rows: Vec<PlanEntryView> = plan
        .entries
        .iter()
        .map(|entry| {
            let copied = entry
                .decisions
                .iter()
                .filter(|d| d.action == DecisionAction::CopyFromOld)
                .count();
            let kept = entry
                .decisions
                .iter()
                .filter(|d| d.action == DecisionAction::KeepNew)
                .count();
            // L4 条目 decisions 为空，动作标签按合并键数表达；空条目明确标示无需处理。
            let action_label = if copied + kept == 0 {
                if entry.total_items > 0 {
                    format!("智能合并 {} 个键位", entry.total_items)
                } else {
                    "无需处理".to_string()
                }
            } else if kept == 0 {
                format!("复制 {copied} 项")
            } else {
                format!("复制 {copied} 项，同名保留新版 {kept} 项")
            };
            PlanEntryView {
                path: entry.rule.relative_path.clone().into(),
                level_label: level_label(entry.rule.level).into(),
                item_count: format!("{} 项", entry.total_items).into(),
                action_label: action_label.into(),
                dimmed: copied + kept == 0 && entry.total_items == 0,
            }
        })
        .collect();
    // 有动作的条目排在前，无动作的淡化条目沉底，减少浏览噪声（稳定排序保持规则顺序）。
    let mut rows = rows;
    rows.sort_by_key(|r| r.dimmed);
    ui.set_plan_entries(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
        rows,
    ))));
    ui.set_plan_summary(plan_summary_text(plan).into());
    ui.set_plan_ready(true);
    ui.set_progress_text("".into());
    set_status(ui, "success", "已生成迁移计划，确认无误后点击「开始迁移」。");
}

/**
 * 函数职责：输出资产级别的用户可读标签。
 * 输入说明：level 为资产分级。
 * 输出说明：固定标签文本。
 * 实现思路：枚举到标签的一一映射。
 */
fn level_label(level: AssetLevel) -> &'static str {
    match level {
        AssetLevel::Direct => "L1 直接复制",
        AssetLevel::Incremental => "L2 增量合并",
        AssetLevel::ModData => "L3 模组数据",
        AssetLevel::SmartMerge => "L4 智能合并",
    }
}

/**
 * 函数职责：生成计划摘要文案（源/目标、规模与被设置跳过的级别）。
 * 输入说明：plan 为 plan_migration 产物。
 * 输出说明：单段用户可读摘要。
 * 实现思路：统计条目与动作数；按选项快照收集关闭项名称。
 */
fn plan_summary_text(plan: &MigrationPlan) -> String {
    let mut skipped: Vec<&str> = Vec::new();
    let o = plan.options;
    if !o.include_saves { skipped.push("存档（L1）"); }
    if !o.include_packs { skipped.push("资源包（L2）"); }
    if !o.include_moddata { skipped.push("模组数据（L3）"); }
    if !o.include_options { skipped.push("客户端偏好（L4）"); }
    let skip_note = if skipped.is_empty() {
        String::new()
    } else {
        format!("；已按设置跳过：{}", skipped.join("、"))
    };
    format!(
        "源 {} → 目标 {} · {} 类资产 · 共 {} 项文件/键位{}",
        plan.source.version.dir_name,
        plan.target.version.dir_name,
        plan.entries.len(),
        plan.total_actions(),
        skip_note
    )
}

/**
 * 函数职责：写入单行状态栏（覆盖式反馈，取代追加日志与结果卡片）。
 * 输入说明：ui 为窗口引用；kind 为 "info"/"success"/"error"；text 为文案。
 * 输出说明：副作用为更新 status-text 与 status-kind。
 * 实现思路：直接写两个属性，颜色由 UI 依据 kind 计算。
 */
fn set_status(ui: &PackPorterWindow, kind: &str, text: &str) {
    ui.set_status_kind(kind.into());
    ui.set_status_text(text.into());
}

/**
 * 函数职责：将后台执行进度回写到 UI 进度条与描述区。
 * 输入说明：weak 为 UI 弱引用（内部克隆以满足事件闭包的 'static 要求）；p 为进度事件。
 * 输出说明：经 invoke_from_event_loop 更新 progress-percent / progress-text。
 * 实现思路：total 为 0 时以不确定态（-1）表示；百分比做上限截断。
 */
fn report_progress(weak: &slint::Weak<PackPorterWindow>, p: &MigrationProgress) {
    let percent = if p.total == 0 {
        -1
    } else {
        ((p.done * 100 / p.total) as i32).min(100)
    };
    let (done, total, current) = (p.done, p.total, p.current.clone());
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_progress_percent(percent);
            ui.set_progress_text(std::format!("({done}/{total}) {current}").into());
        }
    });
}

/**
 * 函数职责：清空计划预览区与进度区（重新扫描或重新生成前旧计划失效）。
 * 输入说明：ui 为窗口引用。
 * 输出说明：副作用为重置 plan-summary、plan-entries 与进度属性。
 * 实现思路：逐属性写回空值/初始值。
 */
fn clear_plan(ui: &PackPorterWindow) {
    ui.set_plan_entries(slint::ModelRc::from(std::rc::Rc::new(
        slint::VecModel::<PlanEntryView>::from(Vec::new()),
    )));
    ui.set_plan_summary("".into());
    ui.set_progress_text("".into());
    ui.set_progress_percent(-1);
}
