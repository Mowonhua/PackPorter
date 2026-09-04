//! 文件职责：UI 交互控制器：将主窗口回调装配到服务层，供桌面入口与集成测试复用。
//! 定义范围：共享状态槽类型、attach 装配函数与预览/日志辅助函数；
//!           目录监控线程不属于交互回调，由入口层自行启动。

use crate::app_config::AppConfig;
use crate::domain::error::PackError;
use crate::domain::instance::{
    AssetLevel, DecisionAction, InstanceProfile, MigrationPlan, MigrationProgress,
};
use crate::services::migration_service::MigrationService;
use crate::{PlanEntryView, PackPorterWindow};
use slint::ComponentHandle;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// 扫描后缓存的实例画像列表（与下拉框模型同源同序）。
type ProfilesSlot = Arc<Mutex<Vec<InstanceProfile>>>;
// 扫描后缓存的迁移编排器，供计划与执行复用。
type ServiceSlot = Arc<Mutex<Option<Arc<Mutex<MigrationService>>>>>;
// 最近一次生成的迁移计划，供执行与打开备份目录复用。
type PlanSlot = Arc<Mutex<Option<MigrationPlan>>>;

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
 * 输入说明：ui 为主窗口引用；versions_root 为扫描用 versions 目录。
 * 输出说明：返回共享状态槽供测试断言；无 panic 路径。
 * 实现思路：注册四个回调——扫描（重建服务并填充下拉框模型、恢复上次选择）、
 *           计划（选中校验 → 占用检测 → 生成并渲染计划）、
 *           执行（后台线程跑事务，进度与结果经事件循环回写）、
 *           打开备份目录（资源管理器打开计划备份路径）。
 */
pub fn attach(ui: &PackPorterWindow, versions_root: PathBuf) -> ControllerHandles {
    let profiles: ProfilesSlot = Arc::new(Mutex::new(Vec::new()));
    let service: ServiceSlot = Arc::new(Mutex::new(None));
    let plan: PlanSlot = Arc::new(Mutex::new(None));

    // 上次迁移选择：扫描成功后尝试恢复下拉框选中项。
    let config = AppConfig::load();
    let last_source = config.last_source;
    let last_target = config.last_target;

    // 配置回填 UI 初始状态。
    ui.set_lock_warning_visible(false);

    // 扫描请求：用装配时给定的 versions 根目录重建服务并填充实例列表。
    let weak_scan = ui.as_weak();
    let root_slot = Arc::new(Mutex::new(versions_root));
    let profiles_slot = profiles.clone();
    let service_slot = service.clone();
    let plan_slot = plan.clone();
    ui.on_scan_requested(move || {
        let ui = weak_scan.unwrap();
        let root = root_slot.lock().unwrap().clone();
        if root.as_os_str().is_empty() {
            append_log(&ui, "未配置 versions 目录，请先在配置文件中设置 versions_dir。");
            return;
        }
        // 每次扫描重建编排器，保证绑定最新的 versions 根目录。
        let new_service = Arc::new(Mutex::new(MigrationService::new(root)));
        let scan_result = new_service.lock().unwrap().instances.scan_instances();
        match scan_result {
            Ok(found) => {
                let count = found.len();
                let names: Vec<slint::SharedString> = found
                    .iter()
                    .map(|p| slint::SharedString::from(p.version.dir_name.as_str()))
                    .collect();
                ui.set_instance_names(slint::ModelRc::from(std::rc::Rc::new(
                    slint::VecModel::from(names),
                )));
                // 新列表与旧选中索引必然失配，重置为未选择并清空旧计划。
                ui.set_source_index(-1);
                ui.set_target_index(-1);
                // 恢复上次迁移选择；源与目标同目录的情况由计划阶段校验兜底。
                if let Some(pos) = found.iter().position(|p| p.version.dir_name == last_source) {
                    ui.set_source_index(pos as i32);
                }
                if let Some(pos) = found.iter().position(|p| p.version.dir_name == last_target) {
                    ui.set_target_index(pos as i32);
                }
                *profiles_slot.lock().unwrap() = found;
                *service_slot.lock().unwrap() = Some(new_service);
                *plan_slot.lock().unwrap() = None;
                clear_plan(&ui);
                append_log(&ui, &format!("扫描完成，发现 {count} 个实例。请选择源实例与目标实例。"));
            }
            Err(e) => append_log(&ui, &format!("扫描失败：{e}")),
        }
    });

    // 计划请求：按选中的源/目标生成迁移计划并填充预览区。
    let weak_plan = ui.as_weak();
    let profiles_plan = profiles.clone();
    let service_plan = service.clone();
    let plan_slot_plan = plan.clone();
    ui.on_plan_requested(move || {
        let ui = weak_plan.unwrap();
        let Some(service_mutex) = service_plan.lock().unwrap().clone() else {
            append_log(&ui, "请先扫描实例。");
            return;
        };
        let profiles = profiles_plan.lock().unwrap();
        let (s, t) = (ui.get_source_index(), ui.get_target_index());
        // 索引有效性：下拉框模型与画像列表同源同序，越界即状态不一致。
        if s < 0 || t < 0 || s as usize >= profiles.len() || t as usize >= profiles.len() {
            append_log(&ui, "请先在下拉框中选择源实例与目标实例。");
            return;
        }
        let source = profiles[s as usize].clone();
        let target = profiles[t as usize].clone();
        drop(profiles);
        if source.root_dir == target.root_dir {
            append_log(&ui, "源实例与目标实例不能相同。");
            return;
        }
        // 占用检测：任一端被运行中的 java 进程占用即阻断计划生成。
        let service = service_mutex.lock().unwrap();
        for (label, profile) in [("源实例", &source), ("目标实例", &target)] {
            if let Err(e) = service.instances.ensure_unlocked(profile) {
                ui.set_lock_warning_visible(true);
                append_log(&ui, &format!("{label}占用检测未通过：{e}"));
                return;
            }
        }
        ui.set_lock_warning_visible(false);
        match service.plan_migration(&source, &target) {
            Ok(p) => {
                apply_plan(&ui, &p);
                *plan_slot_plan.lock().unwrap() = Some(p);
            }
            Err(e) => append_log(&ui, &format!("计划生成失败：{e}")),
        }
    });

    // 迁移执行请求：后台线程执行事务，进度与结果经事件循环回写 UI。
    let weak_exec = ui.as_weak();
    let service_exec = service.clone();
    let plan_exec = plan.clone();
    ui.on_execute_requested(move || {
        let ui = weak_exec.unwrap();
        let Some(plan_snapshot) = plan_exec.lock().unwrap().clone() else {
            append_log(&ui, "请先生成迁移计划。");
            return;
        };
        ui.set_progress_text("迁移执行中，请勿关闭游戏或修改实例目录。".into());
        // 服务槽与弱引用均为 Clone 型句柄：显式克隆后移入后台线程，
        // 避免回调闭包（可多次触发）按引用捕获导致的借用逃逸。
        let service_bg = service_exec.clone();
        let weak_bg = weak_exec.clone();
        std::thread::spawn(move || {
            let Some(service_mutex) = service_bg.lock().unwrap().clone() else {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak_bg.upgrade() {
                        append_log(&ui, "请先扫描实例并生成计划。");
                    }
                });
                return;
            };
            let service = service_mutex.lock().unwrap();
            // 执行阶段进度回调只读捕获弱引用，逐事件回写进度条与描述。
            let result = service.execute_plan(&plan_snapshot, true, &mut |p: MigrationProgress| {
                report_progress(&weak_bg, &p);
            });
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak_bg.upgrade() {
                    match result {
                        Ok(outcome) => {
                            ui.set_progress_percent(100);
                            ui.set_progress_text("迁移完成。".into());
                            append_log(&ui, &outcome.report);
                        }
                        // 事务失败已自动回滚：实例恢复至迁移前状态，需明确告知。
                        Err(PackError::RolledBack { reason, report }) => {
                            ui.set_progress_text("迁移失败，已回滚。".into());
                            ui.set_progress_percent(-1);
                            append_log(&ui, &format!("迁移已回滚：{reason}"));
                            append_log(&ui, &report);
                        }
                        Err(e) => {
                            ui.set_progress_text("迁移失败。".into());
                            ui.set_progress_percent(-1);
                            append_log(&ui, &format!("迁移失败：{e}"));
                        }
                    }
                }
            });
        });
    });

    // 打开备份目录请求：定位最近一次计划的目标实例 backups 目录。
    let weak_open = ui.as_weak();
    let plan_open = plan.clone();
    ui.on_open_backup_folder(move || {
        let ui = weak_open.unwrap();
        let Some(backup_dir) = plan_open.lock().unwrap().as_ref().map(|p| p.backup_dir.clone())
        else {
            append_log(&ui, "请先生成迁移计划（备份目录由目标实例决定）。");
            return;
        };
        open_in_explorer(&ui, &backup_dir);
    });

    ControllerHandles { profiles, service, plan }
}

/**
 * 函数职责：将生成的迁移计划渲染到预览区（摘要 + 逐规则明细行）。
 * 输入说明：ui 为窗口引用；plan 为 plan_migration 产物。
 * 输出说明：副作用为更新 plan-summary、plan-entries 与进度描述属性。
 * 实现思路：逐条目统计复制/保留数量，L4 条目（无文件决策）按合并键数生成动作标签。
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
            }
        })
        .collect();
    ui.set_plan_entries(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
        rows,
    ))));
    ui.set_plan_summary(
        std::format!(
            "源 {} → 目标 {}；共 {} 条规则、覆盖 {} 项文件/键位；备份目录 {}",
            plan.source.version.dir_name,
            plan.target.version.dir_name,
            plan.entries.len(),
            plan.total_actions(),
            plan.backup_dir.display()
        )
        .into(),
    );
    ui.set_progress_text("计划已生成，确认无误后点击「开始迁移」。".into());
    append_log(ui, "迁移计划已生成，请在预览区确认各条目动作。");
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
 * 函数职责：在资源管理器中打开指定目录。
 * 输入说明：ui 为窗口引用；dir 为备份目录绝对路径。
 * 输出说明：副作用为日志区追加打开结果。
 * 实现思路：目录不存在（尚未执行过迁移）时提示；explorer 独立进程打开，失败仅记录。
 */
fn open_in_explorer(ui: &PackPorterWindow, dir: &Path) {
    if !dir.exists() {
        append_log(ui, "备份目录尚未生成（完成一次迁移后自动创建）。");
        return;
    }
    match std::process::Command::new("explorer").arg(dir).spawn() {
        Ok(_) => append_log(ui, &format!("已打开备份目录：{}", dir.display())),
        Err(e) => append_log(ui, &format!("打开备份目录失败：{e}")),
    }
}

/**
 * 函数职责：清空计划预览区与进度区（重新扫描后旧计划失效）。
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
