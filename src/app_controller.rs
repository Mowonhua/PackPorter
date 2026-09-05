//! 文件职责：UI 交互控制器：将主窗口回调装配到服务层，供桌面入口与集成测试复用。
//! 定义范围：共享上下文槽、attach 装配函数、扫描/计划/执行的编排与状态栏反馈辅助。
//! 反馈约定：用户提示统一走单行状态栏（覆盖式），不再使用追加式日志；
//!           扫描与计划生成均在后台线程执行，UI 只在事件循环线程写属性。

use crate::app_config::{AppConfig, UserRuleEntry};
use crate::domain::error::{PackError, PackResult};
use crate::domain::instance::{
    AssetLevel, DecisionAction, InstanceProfile, MigrationPlan, MigrationProgress,
};
use crate::domain::merge::{MergeAction, OptionsMergeMode};
use crate::domain::rules::{normalize_rule_path, rules_conflict};
use crate::services::migration_service::MigrationService;
use crate::{PlanEntryView, PackPorterWindow, RuleEditorApi, RuleRowView};
use slint::{ComponentHandle, Model};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// 扫描后缓存的实例画像列表（与下拉框模型同源同序）。
type ProfilesSlot = Arc<Mutex<Vec<InstanceProfile>>>;
// 扫描后缓存的迁移编排器，供计划与执行复用。
type ServiceSlot = Arc<Mutex<Option<Arc<Mutex<MigrationService>>>>>;
// 最近一次生成的迁移计划，供执行与打开备份目录复用。
type PlanSlot = Arc<Mutex<Option<MigrationPlan>>>;
// 设置页编辑中的规则草稿（打开时从配置装载，保存时写回）。
type WorkingRulesSlot = Arc<Mutex<Vec<UserRuleEntry>>>;

// versions 目录变更时重启目录监控的钩子（由入口层注入，测试可传空实现）。
pub type WatcherRestartHook = Arc<dyn Fn(&str) + Send + Sync>;

/**
 * 接口职责：应用原路径启动器关联或恢复原启动器。
 * 调用方：设置控制器在后台线程持久化新配置后调用。
 * 实现要求：开启时安装所选路径的 shim，关闭时恢复全部原入口；失败返回错误并保持原关联状态。
 */
pub type LauncherSettingsHook = Arc<dyn Fn(bool, &[String]) -> Result<(), String> + Send + Sync>;

/**
 * 接口职责：选择需要关联的启动器 EXE。
 * 调用方：设置控制器在后台线程调用，测试可注入无系统副作用的实现。
 * 实现要求：成功返回启动器路径，取消返回 None；不得写入启动器文件或访问 UI。
 */
pub type LauncherSelectHook = Arc<dyn Fn() -> Result<Option<PathBuf>, String> + Send + Sync>;

/**
 * 函数职责：选择可在原路径接管的启动器 EXE。
 * 输入说明：在后台线程调用，避免阻塞 Slint 事件循环。
 * 输出说明：选择返回路径，取消返回 None；不修改任何文件。
 * 实现思路：原生文件选择器只接受 exe，安装由保存设置统一触发。
 */
fn choose_launcher() -> Result<Option<PathBuf>, String> {
    Ok(rfd::FileDialog::new()
        .set_title("选择 PCL2 / HMCL 启动器 EXE")
        .add_filter("Windows 启动器", &["exe"])
        .pick_file())
}

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
    /// 设置页编辑中的规则草稿（打开时装载、回调更新、保存时写回配置）。
    working_rules: WorkingRulesSlot,
    /// versions 目录变更钩子。
    watcher_hook: WatcherRestartHook,
    /// 后台启动器关联与恢复适配器；UI 不依赖平台实现。
    launcher_hook: LauncherSettingsHook,
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
    // 默认装配不执行启动器系统操作；桌面入口显式注入原生适配器。
    attach_with_launcher_hooks(ui, versions_root, watcher_hook,
        Arc::new(|_, _| Ok(())), Arc::new(|| Ok(None)))
}

/**
 * 函数职责：注入启动器设置适配器并装配原生启动器选择。
 * 输入说明：launcher_hook 不得访问 UI，允许执行阻塞系统调用。
 * 输出说明：返回控制器状态槽；保存失败保留草稿并恢复旧配置。
 * 实现思路：后台保存配置并应用启动器关联，成功后提交生效状态。
 */
pub fn attach_with_launcher_hook(
    ui: &PackPorterWindow,
    versions_root: PathBuf,
    watcher_hook: WatcherRestartHook,
    launcher_hook: LauncherSettingsHook,
) -> ControllerHandles {
    attach_with_launcher_hooks(ui, versions_root, watcher_hook, launcher_hook,
        Arc::new(choose_launcher))
}

/**
 * 函数职责：装配启动器设置与启动器选择适配器。
 * 输入说明：两个适配器均在后台线程调用，不得直接访问 UI。
 * 输出说明：返回共享状态槽；取消选择不改变持久化设置。
 * 实现思路：沿用设置装配，以独立状态标志阻止启动器选择重入及迁移交错。
 */
pub fn attach_with_launcher_hooks(
    ui: &PackPorterWindow,
    versions_root: PathBuf,
    watcher_hook: WatcherRestartHook,
    launcher_hook: LauncherSettingsHook,
    selection_hook: LauncherSelectHook,
) -> ControllerHandles {
    let config = Arc::new(Mutex::new(AppConfig::load()));
    ui.set_launcher_follow_enabled(config.lock().unwrap().follow_launchers);
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
        working_rules: Arc::new(Mutex::new(Vec::new())),
        watcher_hook,
        launcher_hook,
    });

    let weak = ui.as_weak();
    let ctx_select = ctx.clone();
    ui.on_select_launcher(move || {
        let ui = weak.unwrap();
        if ui.get_settings_saving() || ui.get_executing() || ui.get_launcher_selecting() { return; }
        ui.set_launcher_selecting(true);
        set_status(&ui, "info", "请选择要关联的启动器 EXE…");
        let select = selection_hook.clone();
        let ctx_bg = ctx_select.clone();
        let weak_bg = weak.clone();
        // 原生对话框仅产生草稿；保存与页面切换等待其收尾，避免迟到结果污染下一次编辑。
        std::thread::spawn(move || {
            let result = select();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak_bg.upgrade() else { return; };
                ui.set_launcher_selecting(false);
                match result {
                    Ok(Some(path)) => {
                        let mut paths: Vec<String> = ui.get_settings_launcher_paths().iter().map(|p| p.to_string()).collect();
                        let path = path.to_string_lossy().into_owned();
                        if !paths.iter().any(|p| p.eq_ignore_ascii_case(&path)) { paths.push(path.clone()); }
                        set_launcher_paths(&ui, &ctx_bg, paths);
                        set_status(&ui, "success", &format!("已选择 {path}；保存设置后应用关联。"));
                    }
                    Ok(None) => set_status(&ui, "info", "已取消选择启动器。"),
                    Err(error) => set_status(&ui, "error", &format!("启动器选择失败：{error}")),
                }
            });
        });
    });
    let weak = ui.as_weak();
    let ctx_remove = ctx.clone();
    ui.on_remove_launcher(move |index| {
        let ui = weak.unwrap();
        if ui.get_settings_saving() || ui.get_executing() || ui.get_launcher_selecting() { return; }
        let mut paths: Vec<String> = ui.get_settings_launcher_paths().iter().map(|p| p.to_string()).collect();
        if index < 0 || index as usize >= paths.len() { return; }
        paths.remove(index as usize);
        set_launcher_paths(&ui, &ctx_remove, paths);
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
        if ctx_exec.busy.load(Ordering::Relaxed) || ui.get_settings_saving() || ui.get_launcher_selecting() {
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
        if ui.get_settings_saving() || ui.get_launcher_selecting() { return; }
        let config = ctx_settings.config.lock().unwrap();
        ui.set_settings_follow_launchers(config.follow_launchers);
        ui.set_settings_launcher_paths(slint::ModelRc::new(slint::VecModel::from(config.launcher_paths.iter().map(|p| slint::SharedString::from(p.as_str())).collect::<Vec<_>>())));
        ui.set_saved_launcher_paths_dirty(false);
        ui.set_settings_versions_dir(config.versions_dir.clone().into());
        ui.set_settings_auto_backup(config.auto_backup);
        ui.set_settings_include_saves(config.include_saves);
        ui.set_settings_include_packs(config.include_packs);
        ui.set_settings_include_moddata(config.include_moddata);
        ui.set_settings_include_options(config.include_options);
        // 规则草稿从配置装载（未自定义时为内置默认）；弹窗按需加载，无需预同步。
        let working = config.rule_entries();
        // 保存高亮基线：快照已保存值，规则视为无改动。
        ui.set_saved_versions_dir(config.versions_dir.clone().into());
        ui.set_saved_auto_backup(config.auto_backup);
        ui.set_saved_include_saves(config.include_saves);
        ui.set_saved_include_packs(config.include_packs);
        ui.set_saved_include_moddata(config.include_moddata);
        ui.set_saved_include_options(config.include_options);
        ui.set_saved_rules_dirty(false);
        drop(config);
        *ctx_settings.working_rules.lock().unwrap() = working;
        ui.set_settings_open(true);
    });

    let weak = ui.as_weak();
    ui.on_cancel_settings(move || {
        let ui = weak.unwrap();
        if !ui.get_settings_saving() && !ui.get_launcher_selecting() { ui.set_settings_open(false); }
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

    // 规则配置弹窗：按级别装载标题与路径行模型后置位可见。
    let weak = ui.as_weak();
    let ctx_rules = ctx.clone();
    ui.on_open_rule_dialog(move |level| {
        let ui = weak.unwrap();
        let Some(level) = AssetLevel::from_index(level.max(0) as u32) else { return; };
        // 级别属性以 1-4 序号流转（与 UI 回调一致），不能用枚举判别值（0 起始）。
        ui.set_rule_dialog_level(level.index() as i32);
        ui.set_rule_dialog_title(format!("配置迁移路径 · {}", level_label(level)).into());
        let working = ctx_rules.working_rules.lock().unwrap();
        let model = rule_level_model(&working, level);
        drop(working);
        ui.set_rule_dialog_rows(model);
        ui.set_rule_dialog_visible(true);
    });

    // ==================== 规则编辑回调（设置页级别面板，经 RuleEditorApi 桥接） ====================
    let weak = ui.as_weak();
    let ctx_rules = ctx.clone();
    ui.global::<RuleEditorApi>().on_add(move |level, path| -> bool {
        let ui = weak.unwrap();
        let Some(level) = AssetLevel::from_index(level.max(0) as u32) else { return false; };
        let Some(normalized) = validate_rule_input(&ui, &path) else { return false; };
        let mut working = ctx_rules.working_rules.lock().unwrap();
        if let Some(existing) = working
            .iter()
            .find(|e| rules_conflict(&e.relative_path, &normalized))
        {
            set_status(
                &ui,
                "error",
                &format!(
                    "与{}的「{}」互相覆盖，请先删除或修改该规则。",
                    level_label(existing.level),
                    existing.relative_path
                ),
            );
            return false;
        }
        working.push(UserRuleEntry { relative_path: normalized, level, enabled: true });
        sync_rule_models(&ui, &working);
        update_rules_dirty(&ui, &ctx_rules, &working);
        true
    });

    let weak = ui.as_weak();
    let ctx_rules = ctx.clone();
    ui.global::<RuleEditorApi>().on_update(move |level, index, path| -> bool {
        let ui = weak.unwrap();
        let Some(level) = AssetLevel::from_index(level.max(0) as u32) else { return false; };
        let Some(normalized) = validate_rule_input(&ui, &path) else { return false; };
        let mut working = ctx_rules.working_rules.lock().unwrap();
        let Some(flat) = level_entry_index(&working, level, index as usize) else { return false; };
        // 冲突检查需跳过本条目（允许改回自身原路径）。
        if let Some(existing) = working
            .iter()
            .enumerate()
            .find(|(i, e)| *i != flat && rules_conflict(&e.relative_path, &normalized))
            .map(|(_, e)| e)
        {
            set_status(
                &ui,
                "error",
                &format!(
                    "与{}的「{}」互相覆盖，请先删除或修改该规则。",
                    level_label(existing.level),
                    existing.relative_path
                ),
            );
            return false;
        }
        working[flat].relative_path = normalized;
        sync_rule_models(&ui, &working);
        update_rules_dirty(&ui, &ctx_rules, &working);
        true
    });

    let weak = ui.as_weak();
    let ctx_rules = ctx.clone();
    ui.global::<RuleEditorApi>().on_remove(move |level, index| {
        let ui = weak.unwrap();
        let Ok(level) = u32::try_from(level.max(0)) else { return; };
        let Some(level) = AssetLevel::from_index(level) else { return; };
        let mut working = ctx_rules.working_rules.lock().unwrap();
        if let Some(flat) = level_entry_index(&working, level, index as usize) {
            working.remove(flat);
            sync_rule_models(&ui, &working);
            update_rules_dirty(&ui, &ctx_rules, &working);
        }
    });

    let weak = ui.as_weak();
    let ctx_rules = ctx.clone();
    ui.global::<RuleEditorApi>().on_set_enabled(move |level, index, enabled| {
        let ui = weak.unwrap();
        let Ok(level) = u32::try_from(level.max(0)) else { return; };
        let Some(level) = AssetLevel::from_index(level) else { return; };
        let mut working = ctx_rules.working_rules.lock().unwrap();
        if let Some(flat) = level_entry_index(&working, level, index as usize) {
            working[flat].enabled = enabled;
            sync_rule_models(&ui, &working);
            update_rules_dirty(&ui, &ctx_rules, &working);
        }
    });

    let weak = ui.as_weak();
    let ctx_save = ctx.clone();
    ui.on_save_settings(move || {
        let ui = weak.unwrap();
        if ui.get_settings_saving() || ui.get_launcher_selecting() { return; }
        if ctx_save.executing.load(Ordering::Relaxed) {
            set_status(&ui, "info", "迁移执行中，请等待完成后保存设置。");
            return;
        }
        let dir = ui.get_settings_versions_dir().trim().to_string();
        // 未配置 Minecraft 目录也可保存应用设置；非空路径必须是有效目录。
        if !dir.is_empty() && !std::path::Path::new(&dir).is_dir() {
            set_status(&ui, "error", "目录不存在或不可访问，请检查路径后重试。");
            return;
        }
        let previous = ctx_save.config.lock().unwrap().clone();
        let mut next = previous.clone();
        next.versions_dir = dir;
        next.auto_backup = ui.get_settings_auto_backup();
        next.include_saves = ui.get_settings_include_saves();
        next.include_packs = ui.get_settings_include_packs();
        next.include_moddata = ui.get_settings_include_moddata();
        next.include_options = ui.get_settings_include_options();
        next.follow_launchers = ui.get_settings_follow_launchers();
        next.launcher_paths = ui.get_settings_launcher_paths().iter().map(|p| p.to_string()).collect();
        if next.follow_launchers && next.launcher_paths.is_empty() {
            set_status(&ui, "error", "请先选择要关联的 PCL2 / HMCL 启动器。 ");
            return;
        }
        next.rules = Some(ctx_save.working_rules.lock().unwrap().clone());
        ui.set_settings_saving(true);
        set_status(&ui, "info", "正在保存设置…");
        let ctx_bg = ctx_save.clone();
        let weak_bg = weak.clone();
        std::thread::spawn(move || {
            // shim 启动时读取磁盘配置；先落盘，再安装或恢复原入口。
            // 关联变更失败时补偿配置，成功前不发布生效值，保留草稿供重试。
            let result = if !next.save() {
                Err("配置文件写入失败，请检查配置目录权限。".to_string())
            } else if previous.follow_launchers != next.follow_launchers || previous.launcher_paths != next.launcher_paths {
                match (ctx_bg.launcher_hook)(next.follow_launchers, &next.launcher_paths) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        if previous.save() { Err(error) }
                        else { Err(format!("{error}；旧配置恢复失败，请检查配置目录权限。")) }
                    }
                }
            } else { Ok(()) };
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak_bg.upgrade() else { return; };
                ui.set_settings_saving(false);
                if let Err(error) = result {
                    set_status(&ui, "error", &format!("设置保存失败：{error}"));
                    return;
                }
                let dir_changed = previous.versions_dir != next.versions_dir;
                let changes_changed = previous.auto_backup != next.auto_backup
                    || previous.include_saves != next.include_saves
                    || previous.include_packs != next.include_packs
                    || previous.include_moddata != next.include_moddata
                    || previous.include_options != next.include_options
                    || previous.rule_entries() != next.rule_entries();
                {
                    let mut config = ctx_bg.config.lock().unwrap();
                    // 已在途的计划可能刚更新选择；设置提交不能将其回退到提交前快照。
                    next.last_source = config.last_source.clone();
                    next.last_target = config.last_target.clone();
                    *config = next.clone();
                }
                ui.set_launcher_follow_enabled(next.follow_launchers);
                if previous.follow_launchers != next.follow_launchers || previous.launcher_paths != next.launcher_paths {
                    ui.set_launcher_follow_revision(ui.get_launcher_follow_revision().wrapping_add(1));
                }
                ui.set_settings_open(false);
                set_status(&ui, "success", "设置已保存。");
                if dir_changed {
                    *ctx_bg.root_dir.lock().unwrap() = PathBuf::from(&next.versions_dir);
                    (ctx_bg.watcher_hook)(&next.versions_dir);
                    ui.set_has_versions_dir(!next.versions_dir.is_empty());
                    ui.set_plan_ready(false);
                    *ctx_bg.plan.lock().unwrap() = None;
                    *ctx_bg.service.lock().unwrap() = None;
                    ctx_bg.profiles.lock().unwrap().clear();
                    ui.set_instance_names(slint::ModelRc::default());
                    ui.set_source_index(0);
                    ui.set_target_index(0);
                    clear_plan(&ui);
                    if !next.versions_dir.is_empty() { try_start_scan(&ctx_bg, &ui); }
                } else if changes_changed || ctx_bg.replan_pending.swap(false, Ordering::Relaxed) {
                    try_start_plan(&ctx_bg, &ui);
                }
            });
        });
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
    if ui.get_settings_saving() { return false; }
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
        // 每次扫描重建编排器：绑定最新 versions 根目录与当前配置的规则表。
        let rules = ctx_bg.ctx.config.lock().unwrap().effective_registry();
        let service = Arc::new(Mutex::new(MigrationService::with_rules(root.clone(), rules)));
        let scan_result = service.lock().unwrap().instances.scan_instances();
        let _ = slint::invoke_from_event_loop(move || {
            ctx_bg.ctx.busy.store(false, Ordering::Relaxed);
            let Some(ui) = ctx_bg.weak.upgrade() else { return };
            ui.set_busy(false);
            // 设置保存可能在扫描期间切换目录；旧结果不能覆盖新目录的空模型。
            // 原扫描释放 busy 后在此补扫，避免保存时请求因忙碌被丢弃。
            if *ctx_bg.ctx.root_dir.lock().unwrap() != root {
                if ui.get_has_versions_dir() { try_start_scan(&ctx_bg.ctx, &ui); }
                return;
            }
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
                    // 恢复选择可能触发计划生成，先结束扫描提示，避免覆盖后续计划状态。
                    set_status(&ui, "success", &format!("扫描完成，共发现 {count} 个实例。"));
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
    if ui.get_settings_saving() {
        ctx.replan_pending.store(true, Ordering::Relaxed);
        return false;
    }
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
    // 计划前按当前配置刷新规则表：设置页可能已修改迁移路径或启用开关。
    {
        let registry = ctx.config.lock().unwrap().effective_registry();
        service_mutex.lock().unwrap().rules = registry;
    }
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
    let planned_root = ctx.root_dir.lock().unwrap().clone();

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
            // 目录变更优先于旧计划回填；忙碌期间延迟的重扫从此处继续。
            if *ctx_bg.ctx.root_dir.lock().unwrap() != planned_root {
                if ui.get_has_versions_dir() { try_start_scan(&ctx_bg.ctx, &ui); }
                return;
            }
            match plan_result {
                Ok(p) => {
                    apply_plan(&ui, &p);
                    *ctx_bg.ctx.plan.lock().unwrap() = Some(p.clone());
                    // 记住本次选择，下次启动自动恢复。
                    let mut config = ctx_bg.ctx.config.lock().unwrap();
                    config.last_source = p.source.version.dir_name.clone();
                    config.last_target = p.target.version.dir_name.clone();
                    // 保存设置期间磁盘包含待提交快照，最近选择不得覆盖它。
                    if !ui.get_settings_saving() { config.save(); }
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
 * 实现思路：逐条目统计复制/保留数量，L4 条目按处理模式、设置总数与未验证键位数生成动作标签；
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
            // 文件是否缺失由规划时的读取结果决定，预览不可再根据设置数量推断初始化。
            let action_label = if entry.rule.level == AssetLevel::SmartMerge {
                plan.options_results.iter()
                    .find(|outcome| outcome.relative_path == entry.rule.relative_path)
                    .map(|outcome| {
                        let result = &outcome.result;
                        let mode = match result.mode {
                            OptionsMergeMode::Initialize => "初始化个人设置",
                            OptionsMergeMode::Merge => "合并个人设置",
                        };
                        let unverified = result.outcomes.iter()
                            .filter(|item| item.action == MergeAction::TakeUnverifiedBinding)
                            .count();
                        let mut label = format!("{mode} {} 项", result.merged.len());
                        if unverified > 0 {
                            label.push_str(&format!("，未验证键位 {unverified}"));
                        }
                        label
                    })
                    .unwrap_or_else(|| "无需处理".to_string())
            } else if copied + kept == 0 {
                "无需处理".to_string()
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
        "源 {} → 目标 {} · {} 类资产 · 共 {} 项文件/设置{}",
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

// ==================== 规则编辑辅助（UI 线程调用） ====================

/**
 * 函数职责：校验并规范化规则路径输入。
 * 输入说明：ui 为窗口引用（校验失败写状态栏）；raw 为用户原始输入。
 * 输出说明：成功返回规范化路径；失败返回 None（已向状态栏写明原因）。
 * 实现思路：委托领域层 normalize_rule_path，错误文案透传给状态栏。
 */
fn validate_rule_input(ui: &PackPorterWindow, raw: &str) -> Option<String> {
    match normalize_rule_path(raw) {
        Ok(normalized) => Some(normalized),
        Err(reason) => {
            set_status(ui, "error", &reason);
            None
        }
    }
}

/**
 * 函数职责：将"级别内索引"映射为规则草稿的扁平索引。
 * 输入说明：working 为规则草稿；level 为资产级别；index 为该级别面板内的行号。
 * 输出说明：命中返回扁平索引；越界返回 None。
 * 实现思路：按级别过滤计数定位（UI 模型按级别分组展示，草稿按级别顺序扁平存储）。
 */
fn level_entry_index(
    working: &[UserRuleEntry],
    level: AssetLevel,
    index: usize,
) -> Option<usize> {
    let mut seen = 0usize;
    for (flat, entry) in working.iter().enumerate() {
        if entry.level == level {
            if seen == index {
                return Some(flat);
            }
            seen += 1;
        }
    }
    None
}

/**
 * 函数职责：刷新"规则草稿有未保存改动"标记，驱动设置页保存按钮高亮。
 * 输入说明：ui 为窗口引用；ctx 为共享上下文；working 为当前规则草稿
 *           （调用方持有的锁内切片，本函数不得重复加锁，避免同线程死锁）。
 * 输出说明：副作用为写 saved-rules-dirty（草稿与已保存配置逐条比较）。
 * 实现思路：每次规则回调后调用；改动被完全撤销时标记自动清除。
 */
fn update_rules_dirty(ui: &PackPorterWindow, ctx: &ControllerCtx, working: &[UserRuleEntry]) {
    let dirty = *working != ctx.config.lock().unwrap().rule_entries();
    ui.set_saved_rules_dirty(dirty);
}

/**
 * 函数职责：将规则草稿同步到打开中的规则配置弹窗行模型。
 * 输入说明：ui 为窗口引用；working 为规则草稿。
 * 输出说明：副作用为弹窗可见时重写 rule-dialog-rows（当前弹窗级别的行）；
 *           弹窗未打开时为无操作。
 * 实现思路：增删改回调统一走此入口，弹窗行随草稿实时刷新。
 */
fn sync_rule_models(ui: &PackPorterWindow, working: &[UserRuleEntry]) {
    if !ui.get_rule_dialog_visible() {
        return;
    }
    let Some(level) = AssetLevel::from_index(ui.get_rule_dialog_level().max(0) as u32) else {
        return;
    };
    ui.set_rule_dialog_rows(rule_level_model(working, level));
}

/**
 * 函数职责：构建指定级别的规则行模型（弹窗清单数据源）。
 * 输入说明：working 为规则草稿；level 为资产级别。
 * 输出说明：该级别全部规则（含禁用项）的行模型，保持草稿顺序。
 * 实现思路：按级别过滤后映射为 RuleRowView。
 */
fn rule_level_model(working: &[UserRuleEntry], level: AssetLevel) -> slint::ModelRc<RuleRowView> {
    let rows: Vec<RuleRowView> = working
        .iter()
        .filter(|entry| entry.level == level)
        .map(|entry| RuleRowView {
            path: entry.relative_path.clone().into(),
            enabled: entry.enabled,
        })
        .collect();
    slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(rows)))
}

/// 更新启动器草稿与保存提示；磁盘入口只有保存设置时才允许变更。
fn set_launcher_paths(ui: &PackPorterWindow, ctx: &ControllerCtx, paths: Vec<String>) {
    ui.set_saved_launcher_paths_dirty(paths != ctx.config.lock().unwrap().launcher_paths);
    ui.set_settings_launcher_paths(slint::ModelRc::new(slint::VecModel::from(
        paths.into_iter().map(slint::SharedString::from).collect::<Vec<_>>()
    )));
}
