//! 文件职责：UI 交互回调集成测试（回归锁）：驱动真实回调验证扫描/计划/执行全链路。
//! 运行方式：harness = false，以独立程序在主线程运行 Slint 事件循环后自检退出码。
//! 时序说明：扫描与计划生成均为后台任务，测试以单一定时器状态机轮询推进，
//!           全部等待都有超时上限，超时即判失败。

use packporter::app_controller::attach_with_launcher_hooks;
use packporter::app_config::AppConfig;
use packporter::{PackPorterWindow, RuleEditorApi};
use slint::{ComponentHandle, Model};
use std::path::PathBuf;
use std::time::Duration;

fn main() {
    // 主线程先完成全部场景（窗口创建即完成 Slint 后端初始化），
    // 场景内部经事件循环定时器轮询推进，循环返回后统一判定退出码。
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_all_inner));
    let failed = result.is_err();
    if failed {
        eprintln!("UI 回调集成测试失败");
        std::process::exit(1);
    }
    println!("UI 回调集成测试通过");
}

// 轮询状态机的阶段：等待扫描 → 等待计划 → 等待执行完成。
#[derive(PartialEq, Debug, Clone)]
enum Phase {
    Scan,
    Plan,
    Execute,
}

/**
 * 函数职责：测试主体：构建双实例 fixture → 等待自动扫描 → 选中源/目标等待自动计划 →
 *           驱动执行回调并等待结果卡片。
 * 输入说明：无。
 * 输出说明：断言失败即 panic（由 run_all 捕获转退出码）。
 * 实现思路：attach 装配即自动扫描；选择索引写入后由 changed 回调自动出计划；
 *           执行为异步事务，定时器轮询阶段标记直至全部完成退出事件循环。
 */
fn run_all_inner() {
    // 配置重定向到测试私有目录，避免读写真实用户配置。
    let config_dir = std::env::temp_dir().join(format!("packporter_ui_test_cfg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&config_dir);
    std::env::set_var("PACKPORTER_CONFIG_DIR", &config_dir);

    let versions_root = make_fixture();

    let ui = PackPorterWindow::new().expect("创建主窗口失败");
    // 无边框镶边几何：标题栏高度是静态常量，可在此锁定（Rust 命中测试依赖它划分拖动区）；
    // 控制区起点 x 依赖真实窗口布局（未显示时窗口宽为 0），交由静态契约检查与实机冒烟覆盖。
    assert_eq!(ui.get_titlebar_height(), 52.0, "标题栏高度应与自绘顶栏一致");
    let launcher_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let launcher_fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls = launcher_calls.clone();
    let fail = launcher_fail.clone();
    let launcher_selection_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let launcher_selection_outcome = Arc::new(std::sync::Mutex::new(Ok(Some(PathBuf::from("PCL2.exe")))));
    let create_calls = launcher_selection_calls.clone();
    let create_outcome = launcher_selection_outcome.clone();
    let handles = attach_with_launcher_hooks(&ui, versions_root.clone(), Arc::new(|_: &str| {}), Arc::new(move |enabled, paths| {
        calls.lock().unwrap().push((enabled, paths.to_vec()));
        if fail.load(std::sync::atomic::Ordering::Relaxed) { return Err("模拟启动器恢复失败".into()); }
        Ok(())
    }), Arc::new(move || {
        create_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        create_outcome.lock().unwrap().clone()
    }));

    // 设置页开合：open 置位、cancel 复位（顶栏齿轮的开合开关依赖这对回调语义）。
    ui.invoke_open_settings();
    assert!(ui.get_settings_open(), "open-settings 应打开设置页");
    assert!(!ui.get_settings_follow_launchers(), "跟随启动器默认关闭");
    ui.set_settings_follow_launchers(true);
    ui.invoke_cancel_settings();
    assert!(!ui.get_settings_open(), "cancel-settings 应关闭设置页");
    assert!(!AppConfig::load().follow_launchers, "取消不得落盘启动器草稿");
    assert!(launcher_calls.lock().unwrap().is_empty(), "取消不得修改启动器关联");

    // 设置页规则配置弹窗：默认规则装载、新增/编辑/删除与非法输入拒绝。
    ui.invoke_open_settings();
    assert!(!ui.get_saved_rules_dirty(), "打开设置时规则不应标记未保存");
    ui.invoke_open_rule_dialog(1);
    assert!(ui.get_rule_dialog_visible(), "配置回调应打开规则弹窗");
    assert_eq!(ui.get_rule_dialog_rows().row_count(), 4, "L1 默认应有 4 条路径");
    let api = ui.global::<RuleEditorApi>();
    assert!(api.invoke_add(1, "mydata/".into()), "合法新增路径应成功");
    assert_eq!(ui.get_rule_dialog_rows().row_count(), 5, "新增后弹窗行应实时刷新");
    assert!(ui.get_saved_rules_dirty(), "新增规则后应标记未保存改动");
    assert!(!api.invoke_add(1, "mydata".into()), "与现有目录互相覆盖的路径应被拒绝");
    assert!(!api.invoke_add(1, "../escape".into()), "越级路径应被拒绝");
    assert!(!api.invoke_add(1, "  ".into()), "空路径应被拒绝");
    assert!(api.invoke_update(1, 4, "mydata2/".into()), "编辑路径应成功");
    assert_eq!(
        ui.get_rule_dialog_rows().row_data(4).unwrap().path,
        "mydata2/",
        "编辑后行数据应更新"
    );
    api.invoke_remove(1, 4);
    assert_eq!(ui.get_rule_dialog_rows().row_count(), 4, "删除后 L1 应回到 4 条");
    assert!(!ui.get_saved_rules_dirty(), "改动全部撤销后应清除未保存标记");
    api.invoke_set_enabled(1, 0, false);
    assert!(!ui.get_rule_dialog_rows().row_data(0).unwrap().enabled, "禁用后行数据应更新");
    assert!(ui.get_saved_rules_dirty(), "禁用规则后应标记未保存改动");
    api.invoke_set_enabled(1, 0, true);
    assert!(!ui.get_saved_rules_dirty(), "重新启用后未保存标记应清除");
    // 关闭弹窗并取消设置：草稿不落盘。
    ui.set_rule_dialog_visible(false);
    ui.invoke_cancel_settings();
    assert!(AppConfig::load().rules.is_none(), "取消后配置不应写入规则表");

    // 规则草稿在保存设置后持久化到配置文件。
    ui.invoke_open_settings();
    ui.invoke_open_rule_dialog(1);
    assert!(api.invoke_add(1, "persisted/".into()), "持久化用新增路径应成功");
    assert!(api.invoke_add(4, "initialized.txt".into()), "初始化设置规则应成功");
    assert!(api.invoke_add(4, "empty-target.txt".into()), "空目标设置规则应成功");
    ui.set_rule_dialog_visible(false);
    // 配置中 versions 目录为空，保存前需填入合法目录（fixture 根）。
    ui.set_settings_versions_dir(versions_root.to_string_lossy().to_string().into());
    ui.invoke_save_settings();
    wait_for_settings(&ui);
    assert!(!ui.get_settings_open(), "保存后设置页应关闭");
    let saved_rules = AppConfig::load().rules.expect("保存后配置应包含规则表");
    assert!(
        saved_rules.iter().any(|r| r.relative_path == "persisted/"),
        "保存后的规则表应包含新增路径"
    );

    // attach 已对配置过的目录自动扫描：此处等待模型就绪后直接进入计划阶段。
    let weak_ui = ui.as_weak();
    let timer = std::rc::Rc::new(slint::Timer::default());
    let poll = timer.clone();
    let phase = std::cell::RefCell::new(Phase::Scan);
    let ticks = std::cell::Cell::new(0u32);
    let versions_for_assert = versions_root.clone();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(50),
        move || {
            ticks.set(ticks.get() + 1);
            if ticks.get() > 600 {
                poll.stop();
                panic!("测试轮询超时，阶段：{:?}", phase.borrow());
            }
            let ui = weak_ui.unwrap();
            // 先克隆当前阶段再分支：分支内推进阶段需要可变借用。
            let current = phase.borrow().clone();
            match current {
                Phase::Scan => {
                    // 模型首行为占位行，其后才是实例目录名。
                    if ui.get_instance_names().row_count() != 3 {
                        return;
                    }
                    assert_eq!(
                        ui.get_instance_names().row_data(1).unwrap().to_string(),
                        "1.16.5_Old"
                    );
                    assert!(
                        handles.service.lock().unwrap().is_some(),
                        "扫描后应缓存迁移编排器"
                    );
                    assert!(!ui.get_busy(), "扫描完成后忙碌状态应复位");
                    // 选中源/目标：changed 回调应自动触发计划生成。
                    ui.set_source_index(1); // 索引 0 为占位行
                    ui.set_target_index(2);
                    *phase.borrow_mut() = Phase::Plan;
                }
                Phase::Plan => {
                    if !ui.get_plan_ready() {
                        return;
                    }
                    assert!(
                        handles.plan.lock().unwrap().is_some(),
                        "计划回调应缓存迁移计划"
                    );
                    assert!(
                        ui.get_plan_entries().row_count() > 0,
                        "计划预览区应有明细行"
                    );
                    assert!(!ui.get_plan_summary().is_empty(), "计划摘要不应为空");
                    for (path, expected) in [
                        ("options.txt", "合并个人设置 4 项，未验证键位 1"),
                        ("initialized.txt", "初始化个人设置 2 项，未验证键位 1"),
                        ("empty-target.txt", "合并个人设置 1 项"),
                    ] {
                        let row = ui.get_plan_entries().iter().find(|row| row.path == path)
                            .unwrap_or_else(|| panic!("缺少设置预览行：{path}"));
                        assert_eq!(row.action_label, expected, "设置预览应区分目标状态：{path}");
                    }
                    assert!(!ui.get_lock_warning_visible(), "无占用时警告不应显示");
                    assert!(
                        ui.get_status_text().contains("已生成迁移计划"),
                        "计划就绪后状态栏应有指引，实际：{}",
                        ui.get_status_text()
                    );
                    // 执行迁移：异步事务，等待状态栏出现完成标记。
                    ui.invoke_execute_requested();
                    assert!(ui.get_executing(), "执行回调应立即进入执行中状态");
                    *phase.borrow_mut() = Phase::Execute;
                }
                Phase::Execute => {
                    if !ui.get_status_text().contains("迁移完成") {
                        return;
                    }
                    poll.stop();
                    assert!(
                        ui.get_status_kind() == "success",
                        "完成后状态种类应为 success，实际：{}",
                        ui.get_status_kind()
                    );
                    assert!(
                        ui.get_status_text().contains("共复制") && ui.get_status_text().contains("合并设置"),
                        "完成状态应含复制数与合并设置数，实际：{}",
                        ui.get_status_text()
                    );
                    assert_eq!(ui.get_progress_percent(), 100, "完成后进度应为 100%");
                    assert!(!ui.get_busy() && !ui.get_executing(), "完成后忙碌状态应复位");
                    // 文件级结果断言：目标实例落盘内容与备份目录存在性。
                    let target = versions_for_assert.join("1.20.1_New");
                    let options = std::fs::read_to_string(target.join("options.txt")).unwrap();
                    assert!(options.contains("language:zh_CN"), "L4 合并应写入旧值语言键");
                    assert!(options.contains("new_mod_key:2"), "L4 合并应保留新版新增键");
                    assert!(options.contains("key_mod.jump:key.keyboard.r"), "未验证键位应实际保留");
                    let initialized = std::fs::read_to_string(target.join("initialized.txt")).unwrap();
                    assert!(initialized.contains("language:zh_CN"), "缺失目标应初始化个人偏好");
                    assert!(initialized.contains("key_mod.jump:key.keyboard.r"), "初始化应写入未验证键位");
                    assert!(
                        target.join("saves/world1/level.dat").exists(),
                        "L1 存档应已复制"
                    );
                    assert!(
                        target.join("resourcepacks/old_pack.zip").exists(),
                        "L2 旧有新缺的资源包应复制到目标"
                    );
                    // 同名存档覆盖场景：目标内容应变为旧版内容，且备份目录应生成 zip 镜像。
                    let copied = std::fs::read(target.join("saves/world1/level.dat")).unwrap();
                    assert_eq!(copied, b"old-save", "L1 同名覆盖应采用旧版存档");
                    let backups = target.join("backups");
                    assert!(backups.is_dir(), "存在覆盖文件时备份目录应已创建");
                    assert!(
                        std::fs::read_dir(&backups)
                            .unwrap()
                            .filter_map(|e| e.ok())
                            .any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("zip")),
                        "备份目录应包含本次迁移的 zip 镜像"
                    );
                    let _ = slint::quit_event_loop();
                }
            }
        },
    );
    // 全部场景为异步任务：进入事件循环由上方定时器轮询直至完成退出。
    let _ = slint::run_event_loop_until_quit();
    // 循环返回后复验执行结果，保证退出码反映真实事务状态。
    assert!(
        ui.get_status_text().contains("迁移完成") && ui.get_status_kind() == "success",
        "事件循环结束后状态栏应为迁移成功，实际：{}",
        ui.get_status_text()
    );
    ui.invoke_open_settings();
    ui.set_settings_versions_dir("".into());
    ui.set_settings_follow_launchers(true);
    ui.invoke_save_settings();
    assert!(!ui.get_settings_saving(), "开启时必须先选择启动器");
    assert!(ui.get_status_text().contains("选择"));
    ui.invoke_select_launcher();
    wait_for_launcher_selection(&ui);
    assert_eq!(ui.get_settings_launcher_paths().row_count(), 1);
    assert!(AppConfig::load().launcher_paths.is_empty(), "选择仅修改草稿");
    ui.invoke_save_settings();
    assert!(ui.get_settings_saving(), "保存立即进入异步状态");
    ui.invoke_save_settings();
    ui.invoke_cancel_settings();
    ui.invoke_open_settings();
    assert!(ui.get_settings_open() && ui.get_settings_follow_launchers(), "保存期间禁止重复提交和覆盖草稿");
    wait_for_settings(&ui);
    assert!(!ui.get_settings_open());
    assert!(AppConfig::load().follow_launchers && ui.get_launcher_follow_enabled());
    assert!(!ui.get_has_versions_dir() && !ui.get_plan_ready());
    assert_eq!(ui.get_instance_names().row_count(), 0, "清空目录后不得保留旧实例供迁移");
    assert_eq!(ui.get_launcher_follow_revision(), 1);
    assert_eq!(*launcher_calls.lock().unwrap(), vec![(true, vec!["PCL2.exe".to_string()])]);

    launcher_fail.store(true, std::sync::atomic::Ordering::Relaxed);
    ui.invoke_open_settings();
    ui.set_settings_follow_launchers(false);
    ui.invoke_save_settings();
    wait_for_settings(&ui);
    assert!(ui.get_settings_open(), "失败保留设置页和草稿");
    assert!(!ui.get_settings_follow_launchers());
    assert!(AppConfig::load().follow_launchers && ui.get_launcher_follow_enabled(), "失败恢复持久化配置且不更新生效状态");
    assert!(ui.get_status_text().contains("模拟启动器恢复失败"));
    assert_eq!(ui.get_launcher_follow_revision(), 1, "失败不发布配置修订");
    launcher_fail.store(false, std::sync::atomic::Ordering::Relaxed);
    ui.invoke_save_settings();
    wait_for_settings(&ui);
    assert!(!AppConfig::load().follow_launchers && !ui.get_launcher_follow_enabled());
    assert_eq!(ui.get_launcher_follow_revision(), 2);
    assert_eq!(*launcher_calls.lock().unwrap(), vec![(true, vec!["PCL2.exe".to_string()]), (false, vec!["PCL2.exe".to_string()]), (false, vec!["PCL2.exe".to_string()])]);

    ui.invoke_open_settings();
    ui.set_settings_follow_launchers(true);
    ui.set_settings_versions_dir(config_dir.join("missing-versions").to_string_lossy().into_owned().into());
    ui.invoke_save_settings();
    assert!(!ui.get_settings_saving(), "非空无效目录必须拒绝保存");
    assert_eq!(launcher_calls.lock().unwrap().len(), 3);
    ui.set_settings_versions_dir("".into());
    // 将配置目录指向普通文件，稳定模拟不可写目录且不依赖机器 ACL。
    let blocked_dir = config_dir.join("blocked-directory");
    std::fs::write(&blocked_dir, "blocked").unwrap();
    std::env::set_var("PACKPORTER_CONFIG_DIR", &blocked_dir);
    ui.invoke_save_settings();
    wait_for_settings(&ui);
    std::env::set_var("PACKPORTER_CONFIG_DIR", &config_dir);
    assert!(ui.get_settings_open() && ui.get_settings_follow_launchers());
    assert!(!ui.get_launcher_follow_enabled() && !AppConfig::load().follow_launchers);
    assert!(ui.get_status_text().contains("配置文件写入失败"));
    assert_eq!(launcher_calls.lock().unwrap().len(), 3, "写盘失败不得修改启动器关联");
    ui.set_settings_saving(true);
    ui.invoke_select_launcher();
    assert!(!ui.get_launcher_selecting(), "保存期间不得打开启动器选择对话框");
    ui.set_settings_saving(false);
    ui.set_executing(true);
    ui.invoke_select_launcher();
    assert!(!ui.get_launcher_selecting(), "迁移期间不得选择启动器");
    ui.set_executing(false);
    ui.invoke_select_launcher();
    assert!(ui.get_launcher_selecting());
    ui.invoke_select_launcher();
    wait_for_launcher_selection(&ui);
    assert_eq!(launcher_selection_calls.load(std::sync::atomic::Ordering::Relaxed), 2, "选择期间禁止重复触发");
    assert!(ui.get_status_text().contains("PCL2.exe"));
    assert!(ui.get_status_text().contains("保存"), "草稿开启不等于配置已生效");
    assert!(!AppConfig::load().follow_launchers, "选择启动器不得改动联动配置");
    *launcher_selection_outcome.lock().unwrap() = Ok(None);
    ui.invoke_select_launcher();
    wait_for_launcher_selection(&ui);
    assert!(ui.get_status_text().contains("已取消"));
    *launcher_selection_outcome.lock().unwrap() = Err("模拟启动器选择失败".into());
    ui.invoke_select_launcher();
    wait_for_launcher_selection(&ui);
    assert!(ui.get_status_text().contains("模拟启动器选择失败"));
    assert_eq!(ui.get_settings_launcher_paths().row_count(), 1, "重复选择、取消和失败不得重复添加路径");
    *launcher_selection_outcome.lock().unwrap() = Ok(Some(PathBuf::from("HMCL.exe")));
    ui.invoke_select_launcher();
    wait_for_launcher_selection(&ui);
    assert_eq!(ui.get_settings_launcher_paths().row_count(), 2, "支持多个启动器");
    assert!(ui.get_saved_launcher_paths_dirty());
    ui.invoke_remove_launcher(0);
    assert_eq!(ui.get_settings_launcher_paths().row_data(0).unwrap(), "HMCL.exe");
    ui.invoke_cancel_settings();
    ui.invoke_open_settings();
    assert_eq!(ui.get_settings_launcher_paths().row_data(0).unwrap(), "PCL2.exe", "取消放弃路径增删草稿");
    assert!(!ui.get_saved_launcher_paths_dirty());
    assert_eq!(launcher_calls.lock().unwrap().len(), 3, "草稿选择和移除不触发安装或还原");
    ui.invoke_select_launcher();
    wait_for_launcher_selection(&ui);
    ui.invoke_save_settings();
    wait_for_settings(&ui);
    assert_eq!(AppConfig::load().launcher_paths, vec!["PCL2.exe", "HMCL.exe"]);
    assert_eq!(launcher_calls.lock().unwrap().last().unwrap(), &(false, vec!["PCL2.exe".into(), "HMCL.exe".into()]), "关闭时保存路径也调用同步钩子");

    let _ = slint::quit_event_loop();
}

// 在真实事件循环等待原生选择适配器收尾，避免只验证后台派发而遗漏 UI 结果。
fn wait_for_launcher_selection(ui: &PackPorterWindow) {
    let weak = ui.as_weak();
    let ticks = std::cell::Cell::new(0);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(10), move || {
        ticks.set(ticks.get() + 1);
        assert!(ticks.get() < 500, "启动器选择超时");
        if !weak.unwrap().get_launcher_selecting() { let _ = slint::quit_event_loop(); }
    });
    slint::run_event_loop_until_quit().unwrap();
}

// 在真实事件循环等待保存收尾；超时使回调测试失败，避免后台失败被误判成功。
fn wait_for_settings(ui: &PackPorterWindow) {
    let weak = ui.as_weak();
    let ticks = std::cell::Cell::new(0);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(10), move || {
        ticks.set(ticks.get() + 1);
        assert!(ticks.get() < 500, "设置保存超时");
        if !weak.unwrap().get_settings_saving() { let _ = slint::quit_event_loop(); }
    });
    slint::run_event_loop_until_quit().unwrap();
}

use std::sync::Arc;

/**
 * 函数职责：在系统临时目录构建两个 Minecraft 实例目录（旧版有存档/资源包/options，新版少量文件）。
 * 输入说明：无。
 * 输出说明：返回 versions 根目录路径；目录残留时先清空重建。
 * 实现思路：手写临时目录（不引入 tempfile 依赖），按 L1/L2/L4 语义放置夹具文件。
 */
fn make_fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("packporter_ui_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let old = root.join("1.16.5_Old");
    let new = root.join("1.20.1_New");
    std::fs::create_dir_all(old.join("saves/world1")).unwrap();
    std::fs::create_dir_all(old.join("resourcepacks")).unwrap();
    std::fs::create_dir_all(new.join("resourcepacks")).unwrap();
    std::fs::create_dir_all(new.join("saves/world1")).unwrap();

    std::fs::write(old.join("saves/world1/level.dat"), "old-save").unwrap();
    // 目标同名存档：验证 L1 覆盖路径同时触发迁移前 Zip 备份。
    std::fs::write(new.join("saves/world1/level.dat"), "new-save").unwrap();
    std::fs::write(old.join("resourcepacks/old_pack.zip"), "oldpack").unwrap();
    std::fs::write(
        old.join("options.txt"),
        "language:zh_CN\nsoundCategory_master:0.5\nold_mod_key:1\nkey_mod.jump:key.keyboard.r\n",
    )
    .unwrap();
    std::fs::write(new.join("options.txt"), "language:en_US\nnew_mod_key:2\n").unwrap();
    std::fs::write(old.join("initialized.txt"), "language:zh_CN\nkey_mod.jump:key.keyboard.r\n").unwrap();
    std::fs::write(old.join("empty-target.txt"), "language:zh_CN\n").unwrap();
    std::fs::write(new.join("empty-target.txt"), "").unwrap();

    // 版本 json：让扫描产出可读的 MC 版本号（缺失也不影响扫描）。
    std::fs::write(old.join("1.16.5_Old.json"), r#"{"minecraft_version":"1.16.5"}"#).unwrap();
    std::fs::write(new.join("1.20.1_New.json"), r#"{"minecraft_version":"1.20.1"}"#).unwrap();
    root
}
