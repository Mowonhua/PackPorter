//! 文件职责：UI 交互回调集成测试（回归锁）：驱动真实回调验证扫描/计划/执行全链路。
//! 运行方式：harness = false，以独立程序在主线程运行 Slint 事件循环后自检退出码。

use packporter::app_controller::attach;
use packporter::PackPorterWindow;
use slint::{ComponentHandle, Model};
use std::path::PathBuf;
use std::time::Duration;

fn main() {
    // 主线程先执行同步场景（窗口创建即完成 Slint 后端初始化），
    // 场景三在其尾部进入事件循环，等待后台迁移完成后再返回统一判定退出码。
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_all_inner));
    let failed = result.is_err();
    if failed {
        eprintln!("UI 回调集成测试失败");
        std::process::exit(1);
    }
    println!("UI 回调集成测试通过");
}

/**
 * 函数职责：测试主体：构建双实例 fixture → 驱动扫描/计划回调 → 驱动执行回调并轮询完成。
 * 输入说明：无。
 * 输出说明：断言失败即 panic（由 run_all 捕获转退出码）。
 * 实现思路：同步回调直接调用 run_*；执行回调为异步事务，用事件循环 Timer 轮询日志完成标记。
 */
fn run_all_inner() {
    let versions_root = make_fixture();

    let ui = PackPorterWindow::new().expect("创建主窗口失败");
    let handles = attach(&ui, versions_root.clone());

    // ===== 场景一：扫描回调填充下拉框模型并恢复状态 =====
    ui.invoke_scan_requested();
    assert_eq!(
        ui.get_instance_names().row_count(),
        2,
        "扫描后下拉框模型应有 2 个实例"
    );
    assert_eq!(
        ui.get_instance_names().row_data(0).unwrap().to_string(),
        "1.16.5_Old"
    );
    assert!(
        handles.service.lock().unwrap().is_some(),
        "扫描后应缓存迁移编排器"
    );

    // ===== 场景二：计划回调生成计划并渲染预览 =====
    // 双向绑定下直接写选中索引等价于用户在下拉框中选中对应项。
    ui.set_source_index(0);
    ui.set_target_index(1);
    ui.invoke_plan_requested();
    assert!(
        handles.plan.lock().unwrap().is_some(),
        "计划回调应缓存迁移计划"
    );
    assert!(
        ui.get_plan_entries().row_count() > 0,
        "计划预览区应有明细行"
    );
    assert!(
        !ui.get_plan_summary().is_empty(),
        "计划摘要不应为空"
    );
    assert!(
        !ui.get_lock_warning_visible(),
        "无占用时警告不应显示"
    );

    // ===== 场景三：执行回调在后台完成事务并回写 UI =====
    ui.invoke_execute_requested();
    let weak_ui = ui.as_weak();
    let timer = std::rc::Rc::new(slint::Timer::default());
    let poll_timer = timer.clone();
    let mut ticks = 0u32;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(50),
        move || {
            ticks += 1;
            let ui = weak_ui.unwrap();
            let log = ui.get_log_text().to_string();
            if log.contains("迁移完成") || log.contains("已回滚") || log.contains("迁移失败") {
                poll_timer.stop();
                assert!(
                    log.contains("迁移完成"),
                    "事务应成功完成，实际日志：{log}"
                );
                assert_eq!(ui.get_progress_percent(), 100, "完成后进度应为 100%");
                // 文件级结果断言：目标实例落盘内容与备份目录存在性。
                let target = versions_root.join("1.20.1_New");
                let options = std::fs::read_to_string(target.join("options.txt")).unwrap();
                assert!(options.contains("language:zh_CN"), "L4 合并应写入旧值语言键");
                assert!(options.contains("new_mod_key:2"), "L4 合并应保留新版新增键");
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
            } else if ticks > 600 {
                poll_timer.stop();
                panic!("执行回调超时未完成，日志：{log}");
            }
        },
    );
    // 场景三为异步事务：进入事件循环由上方 Timer 轮询直至完成退出。
    let _ = slint::run_event_loop_until_quit();
    // 循环返回后复验执行结果，保证退出码反映真实事务状态。
    let log = ui.get_log_text().to_string();
    assert!(log.contains("迁移完成"), "事件循环结束后日志应含迁移完成：{log}");
    assert_eq!(ui.get_progress_percent(), 100, "完成后进度应为 100%");
}

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
        "language:zh_CN\nsoundCategory_master:0.5\nold_mod_key:1\n",
    )
    .unwrap();
    std::fs::write(new.join("options.txt"), "language:en_US\nnew_mod_key:2\n").unwrap();

    // 版本 json：让扫描产出可读的 MC 版本号（缺失也不影响扫描）。
    std::fs::write(old.join("1.16.5_Old.json"), r#"{"minecraft_version":"1.16.5"}"#).unwrap();
    std::fs::write(new.join("1.20.1_New.json"), r#"{"minecraft_version":"1.20.1"}"#).unwrap();
    root
}
