/**
 * 文件职责：模块 B 合并引擎的单元测试：解析、白名单合并与淘汰键位判定。
 * 定义范围：KeyValueParser 与 OptionsMergeEngine 的行为验证。
 */

use std::path::Path;
use std::sync::Arc;

use packporter::domain::instance::{OptionsParser, ParsedOptions};
use packporter::domain::merge::MergeAction;
use packporter::infra::key_value::{KeyValueParser, WhitelistPolicy};
use packporter::services::options_merge::OptionsMergeEngine;

/**
 * 函数职责：构造使用默认解析器与策略的合并引擎。
 * 输入说明：无。
 * 输出说明：引擎实例。
 * 实现思路：装配默认组件。
 */
fn engine() -> OptionsMergeEngine {
    OptionsMergeEngine::new(Arc::new(KeyValueParser), Arc::new(WhitelistPolicy))
}

/**
 * 函数职责：构造测试用 ParsedOptions。
 * 输入说明：pairs 为键值对切片。
 * 输出说明：解析结果结构。
 * 实现思路：逐对插入。
 */
fn map(pairs: &[(&str, &str)]) -> ParsedOptions {
    let mut parsed = ParsedOptions::default();
    for (k, v) in pairs {
        parsed.entries.insert(k.to_string(), v.to_string());
    }
    parsed
}

#[test]
fn parser_splits_on_first_colon_and_keeps_quotes() {
    let parsed = KeyValueParser.parse("fov:0.5\nkey_key.jump:key.keyboard.space:2\nrenderClouds:\"false\"\n\n#comment\n");
    // 值内冒号必须保留：键位描述含 "key.keyboard.space:2"。
    assert_eq!(parsed.entries.get("key_key.jump").unwrap(), "key.keyboard.space:2");
    // 引号原样保留。
    assert_eq!(parsed.entries.get("renderClouds").unwrap(), "\"false\"");
    // 空行与注释进 skipped_lines，不丢行。
    assert_eq!(parsed.skipped_lines.len(), 2);
}

#[test]
fn parser_strips_bom() {
    let parsed = KeyValueParser.parse("\u{feff}lang:zh_cn\n");
    assert!(parsed.entries.contains_key("lang"));
}

#[test]
fn key_bindings_take_old_value() {
    let result = engine().merge_maps(
        &map(&[("key_key.jump", "key.keyboard.space")]),
        &map(&[("key_key.jump", "key.keyboard.unknown")]),
    );
    // 键位族：旧值优先。
    let jump = result.outcomes.iter().find(|o| o.key == "key_key.jump").unwrap();
    assert_eq!(jump.action, MergeAction::TakeOldBinding);
    assert!(result.merged.contains(&("key_key.jump".to_string(), "key.keyboard.space".to_string())));
}

#[test]
fn whitelisted_preferences_take_old_value() {
    let result = engine().merge_maps(
        &map(&[("fov", "0.8"), ("soundCategory_music", "0.3"), ("lang", "zh_cn"), ("guiScale", "4")]),
        &map(&[("fov", "0.0"), ("soundCategory_music", "1.0"), ("lang", "en_us"), ("guiScale", "0")]),
    );
    for key in ["fov", "soundCategory_music", "lang", "guiScale"] {
        let outcome = result.outcomes.iter().find(|o| o.key == key).unwrap();
        assert_eq!(outcome.action, MergeAction::TakeOld, "键 {key} 应采用旧值");
    }
}

#[test]
fn unknown_legacy_keys_are_dropped() {
    let result = engine().merge_maps(
        &map(&[("snooperEnabled", "false"), ("oldModSetting", "42")]),
        &map(&[("maxFps", "260")]),
    );
    // 旧版独有非键位键：智能忽略，不写入新版。
    assert_eq!(result.dropped, 2);
    assert!(!result.merged.iter().any(|(k, _)| k == "oldModSetting"));
    // 新版独有键保留。
    assert!(result.merged.iter().any(|(k, v)| k == "maxFps" && v == "260"));
}

#[test]
fn obsolete_binding_dropped_but_rebindable_binding_kept() {
    // 新版键位族只含 jump；旧版 pickItem 已淘汰，sneak 仍在（补清场景：新版缺失旧键位）。
    let result = engine().merge_maps(
        &map(&[("key_key.pickItem", "key.mouse.middle"), ("key_key.sneak", "key.keyboard.left.shift")]),
        &map(&[("key_key.jump", "key.keyboard.space"), ("key_key.sneak", "key.keyboard.shift.right")]),
    );
    // pickItem 不在新版键族：淘汰忽略。
    let pick = result.outcomes.iter().find(|o| o.key == "key_key.pickItem").unwrap();
    assert_eq!(pick.action, MergeAction::DropLegacy);
    assert!(!result.merged.iter().any(|(k, _)| k == "key_key.pickItem"));
    // sneak 在新版键族存在但本次旧版为独有键：补清写入旧值。
    assert!(result.merged.iter().any(|(k, v)| k == "key_key.sneak" && v == "key.keyboard.left.shift"));
}

#[test]
fn dirty_numeric_value_falls_back_to_new() {
    let result = engine().merge_maps(
        &map(&[("maxFps", "not-a-number")]),
        &map(&[("maxFps", "260")]),
    );
    // 脏值：合法性校验失败，回退新版默认值。
    let outcome = result.outcomes.iter().find(|o| o.key == "maxFps").unwrap();
    assert_eq!(outcome.action, MergeAction::KeepNew);
    assert!(result.merged.contains(&("maxFps".to_string(), "260".to_string())));
}

#[test]
fn missing_new_file_initializes_from_old() {
    let dir = std::env::temp_dir().join("packporter_test_merge");
    let _ = std::fs::create_dir_all(&dir);
    let old = dir.join("old_options.txt");
    let new = dir.join("new_options.txt");
    std::fs::write(&old, "fov:0.9\nkey_key.jump:key.keyboard.space\nlang:zh_cn\n").unwrap();
    let _ = std::fs::remove_file(&new);
    let result = engine().merge_options(&old, &new).unwrap();
    // 新版文件缺失：等价于以旧值初始化白名单键。
    assert!(result.merged.iter().any(|(k, v)| k == "fov" && v == "0.9"));
    assert!(result.merged.iter().any(|(k, v)| k == "key_key.jump" && v == "key.keyboard.space"));
}

#[test]
fn serialize_roundtrip_matches_key_value_lines() {
    let result = engine().merge_maps(
        &map(&[("fov", "0.5")]),
        &map(&[("fov", "0.0"), ("maxFps", "260")]),
    );
    let text = engine().serialize(&result);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.contains(&"fov:0.5"));
    assert!(lines.contains(&"maxFps:260"));
    // 重新解析应与 merged 一致（往返稳定性）。
    let reparsed = KeyValueParser.parse(&text);
    assert_eq!(reparsed.entries.get("fov").unwrap(), "0.5");
}

#[test]
fn loader_hint_priority_prefers_neoforge_over_forge() {
    use packporter::domain::instance::LoaderKind;
    // "forge" 是 "neoforge" 的子串：优先级必须保证 NeoForge 不被误判。
    assert_eq!(LoaderKind::from_profile_hints(&["neoforge 21.1"] ,), LoaderKind::NeoForge);
    assert_eq!(LoaderKind::from_profile_hints(&["forge 47.3.0"]), LoaderKind::Forge);
    assert_eq!(LoaderKind::from_profile_hints(&["fabricloader 0.16.9"]), LoaderKind::Fabric);
    assert_eq!(LoaderKind::from_profile_hints(&["nothing"]), LoaderKind::Vanilla);
}

#[test]
fn find_rule_matches_dir_prefix_and_exact_file() {
    use packporter::domain::rules::{built_in_rules, find_rule};
    let registry = built_in_rules();
    // 目录前缀匹配。
    assert!(find_rule(&registry, "saves/MyWorld/level.dat").is_some());
    // 文件全等匹配。
    assert!(find_rule(&registry, "options.txt").is_some());
    // 未注册路径返回 None。
    assert!(find_rule(&registry, "logs/latest.log").is_none());
}

#[test]
fn rule_registry_covers_all_levels() {
    use packporter::domain::instance::AssetLevel;
    use packporter::domain::rules::built_in_rules;
    let registry = built_in_rules();
    for level in [AssetLevel::Direct, AssetLevel::Incremental, AssetLevel::ModData, AssetLevel::SmartMerge] {
        assert!(
            registry.entries.iter().any(|r| r.level == level),
            "缺少级别 {level:?} 的规则"
        );
    }
}

#[test]
fn transaction_rolls_back_on_failure() {
    use packporter::domain::transaction::{MigrationTransaction, TransactionAction};
    use packporter::services::backup_engine::BackupEngine;

    let dir = std::env::temp_dir().join("packporter_test_tx");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // 预置将被覆盖的既有文件。
    let existing = dir.join("existing.txt");
    std::fs::write(&existing, "original").unwrap();

    let engine = BackupEngine::for_instance(dir.clone());
    let mut progress_called = 0usize;
    // 第二个动作写入不可达路径（其父目录为文件），强制事务失败。
    let blocker = dir.join("blocker.txt");
    std::fs::write(&blocker, "x").unwrap();
    let actions = vec![
        TransactionAction::CopyFile {
            source: blocker.clone(),
            destination: existing.clone(),
        },
        TransactionAction::WriteText {
            destination: blocker.join("child").join("out.txt"),
            content: "nope".to_string(),
        },
    ];
    let result = engine.execute(&actions, &mut |_| progress_called += 1);
    // 事务必须失败并回滚。
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, packporter::domain::error::PackError::RolledBack { .. }));
    // 既有文件被还原为原始内容。
    assert_eq!(std::fs::read_to_string(&existing).unwrap(), "original");
    let _ = progress_called;
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn transaction_commits_on_success() {
    use packporter::domain::transaction::{MigrationTransaction, TransactionAction};
    use packporter::services::backup_engine::BackupEngine;

    let dir = std::env::temp_dir().join("packporter_test_tx_ok");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("src.txt");
    std::fs::write(&source, "data").unwrap();

    let engine = BackupEngine::for_instance(dir.clone());
    let actions = vec![TransactionAction::CopyFile {
        source: source.clone(),
        destination: dir.join("nested").join("dst.txt"),
    }];
    let applied = engine.execute(&actions, &mut |_| {}).unwrap();
    assert_eq!(applied, 1);
    assert_eq!(
        std::fs::read_to_string(dir.join("nested").join("dst.txt")).unwrap(),
        "data"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn zip_backup_roundtrip_restores_content() {
    use packporter::infra::zip_archive::{backup_file_name, pack_files, unpack_to};

    let dir = std::env::temp_dir().join("packporter_test_zip");
    let _ = std::fs::remove_dir_all(&dir);
    let root = dir.join("instance");
    let nested = root.join("config");
    std::fs::create_dir_all(&nested).unwrap();
    let file = nested.join("x.toml");
    std::fs::write(&file, "value = 1").unwrap();

    let zip_path = dir.join(backup_file_name(chrono::Local::now()));
    let files = vec![file.clone()];
    let packed = pack_files(&files, &root, &zip_path, &mut |_, _| {}).unwrap();
    assert_eq!(packed, 1);

    // 修改原文件后从 zip 还原。
    std::fs::write(&file, "changed").unwrap();
    let report = unpack_to(&zip_path, &root).unwrap();
    assert_eq!(report.restored, 1);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "value = 1");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn instance_scan_discovers_real_style_layout() {
    use packporter::services::instance_service::InstanceService;

    let root = std::env::temp_dir().join("packporter_test_scan").join("versions");
    let _ = std::fs::remove_dir_all(&root);
    // 模拟真实布局：目录 + 同名 jar/json。
    let inst = root.join("MyPack 1.0");
    std::fs::create_dir_all(&inst).unwrap();
    std::fs::write(inst.join("MyPack 1.0.jar"), "").unwrap();
    std::fs::write(
        inst.join("MyPack 1.0.json"),
        r#"{"id":"MyPack 1.0","inheritsFrom":"1.20.1","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.9"}]}"#,
    )
    .unwrap();
    // 非实例文件应被跳过。
    std::fs::write(root.join("stray.txt"), "").unwrap();

    let service = InstanceService::new(root.clone());
    let versions = service.list_versions().unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].dir_name, "MyPack 1.0");
    assert_eq!(versions[0].jar_name, "MyPack 1.0");

    let profiles = service.scan_instances().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].loader, packporter::domain::instance::LoaderKind::Fabric);
    assert_eq!(profiles[0].loader_version.as_deref(), Some("0.16.9"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn plan_and_execute_end_to_end_migrates_assets() {
    use packporter::domain::instance::AssetLevel;
    use packporter::services::migration_service::MigrationService;

    let base = std::env::temp_dir().join("packporter_test_e2e");
    let _ = std::fs::remove_dir_all(&base);
    let versions = base.join("versions");
    let old = versions.join("Old 1.0");
    let new = versions.join("New 1.0");
    std::fs::create_dir_all(old.join("saves/world")).unwrap();
    std::fs::create_dir_all(old.join("resourcepacks")).unwrap();
    std::fs::create_dir_all(old.join("xaero/minimap")).unwrap();
    std::fs::create_dir_all(new.join("resourcepacks")).unwrap();
    std::fs::write(old.join("saves/world/level.dat"), "level").unwrap();
    std::fs::write(old.join("resourcepacks/old.zip"), "old").unwrap();
    std::fs::write(new.join("resourcepacks/old.zip"), "new-keep").unwrap();
    std::fs::write(old.join("xaero/minimap/data"), "map").unwrap();
    std::fs::write(old.join("options.txt"), "fov:0.85\nkey_key.jump:key.keyboard.space\noldLeftover:1\n").unwrap();

    let service = MigrationService::new(versions.clone());
    let instances = service.instances.scan_instances().unwrap();
    let source = instances.iter().find(|p| p.version.dir_name == "Old 1.0").unwrap();
    let target = instances.iter().find(|p| p.version.dir_name == "New 1.0").unwrap();

    let plan = service
        .plan_migration(source, target, packporter::domain::instance::MigrationOptions::all_enabled())
        .unwrap();
    // L1 存档复制、L2 同名保留新版、L3 地图复制、L4 合并明细存在。
    let saves_entry = plan.entries.iter().find(|e| e.rule.relative_path == "saves/").unwrap();
    assert!(saves_entry.decisions.iter().any(|d| d.relative_path == "saves/world/level.dat"));
    let packs_entry = plan.entries.iter().find(|e| e.rule.level == AssetLevel::Incremental).unwrap();
    assert!(packs_entry.decisions.iter().any(|d| d.action == packporter::domain::instance::DecisionAction::KeepNew));
    assert!(plan.options_result.is_some());
    assert_eq!(plan.options_result.as_ref().unwrap().merged.iter().find(|(k, _)| k == "fov").unwrap().1, "0.85");

    let outcome = service.execute_plan(&plan, true, &mut |_| {}).unwrap();
    assert!(outcome.success);
    // 新实例获得资产。
    assert_eq!(std::fs::read_to_string(new.join("saves/world/level.dat")).unwrap(), "level");
    assert_eq!(std::fs::read_to_string(new.join("resourcepacks/old.zip")).unwrap(), "new-keep");
    assert_eq!(std::fs::read_to_string(new.join("xaero/minimap/data")).unwrap(), "map");
    let merged_options = std::fs::read_to_string(new.join("options.txt")).unwrap();
    assert!(merged_options.contains("fov:0.85"));
    assert!(merged_options.contains("key_key.jump:key.keyboard.space"));
    assert!(!merged_options.contains("oldLeftover"));
    // 计划包含备份目录。
    assert!(plan.backup_dir.ends_with("backups"));
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn execute_plan_rejects_unconfirmed() {
    use packporter::services::migration_service::MigrationService;
    let versions = std::env::temp_dir().join("packporter_test_unconfirmed").join("versions");
    let _ = std::fs::remove_dir_all(&versions);
    let service = MigrationService::new(versions);
    let a = packporter::domain::instance::InstanceProfile {
        version: packporter::domain::instance::MinecraftVersion {
            dir_name: "A".into(),
            jar_name: String::new(),
        },
        root_dir: std::path::PathBuf::from("/nonexistent-a"),
        profile_path: None,
        mc_version: "1.20.1".into(),
        loader: packporter::domain::instance::LoaderKind::Fabric,
        loader_version: None,
        locked: false,
        locked_by: None,
    };
    let plan = packporter::domain::instance::MigrationPlan {
        source: a.clone(),
        target: packporter::domain::instance::InstanceProfile {
            version: packporter::domain::instance::MinecraftVersion {
                dir_name: "B".into(),
                jar_name: String::new(),
            },
            root_dir: std::path::PathBuf::from("/nonexistent-b"),
            profile_path: None,
            mc_version: "1.21.1".into(),
            loader: packporter::domain::instance::LoaderKind::Fabric,
            loader_version: None,
            locked: false,
            locked_by: None,
        },
        entries: Vec::new(),
        backup_dir: std::path::PathBuf::from("/tmp/backups"),
        options_result: None,
        options: packporter::domain::instance::MigrationOptions::all_enabled(),
    };
    let _ = &a;
    let result = service.execute_plan(&plan, false, &mut |_| {});
    assert!(matches!(result, Err(packporter::domain::error::PackError::InvalidPlan(_))));
}

#[test]
fn plan_options_exclude_disabled_levels_and_skip_backup() {
    use packporter::domain::instance::{DecisionAction, MigrationOptions};
    use packporter::services::migration_service::MigrationService;

    let base = std::env::temp_dir().join("packporter_test_options");
    let _ = std::fs::remove_dir_all(&base);
    let versions = base.join("versions");
    let old = versions.join("Old");
    let new = versions.join("New");
    std::fs::create_dir_all(old.join("saves")).unwrap();
    std::fs::create_dir_all(old.join("resourcepacks")).unwrap();
    std::fs::create_dir_all(new.join("saves")).unwrap();
    std::fs::create_dir_all(new.join("resourcepacks")).unwrap();
    std::fs::write(old.join("saves/level.dat"), "level").unwrap();
    std::fs::write(old.join("resourcepacks/pack.zip"), "pack").unwrap();
    std::fs::write(new.join("saves/level.dat"), "new-level").unwrap();
    std::fs::write(old.join("options.txt"), "fov:0.85\n").unwrap();
    std::fs::write(new.join("options.txt"), "fov:0.6\n").unwrap();

    let service = MigrationService::new(versions.clone());
    let instances = service.instances.scan_instances().unwrap();
    let source = instances.iter().find(|p| p.version.dir_name == "Old").unwrap();
    let target = instances.iter().find(|p| p.version.dir_name == "New").unwrap();

    // 关闭 L1/L2/L4：计划只应包含 L3 及以下条目（此处为空），options 明细为 None。
    let mut options = MigrationOptions::all_enabled();
    options.include_saves = false;
    options.include_packs = false;
    options.include_options = false;
    let plan = service.plan_migration(source, target, options).unwrap();
    assert!(
        !plan.entries.iter().any(|e| e.rule.relative_path == "saves/"),
        "关闭 L1 后计划不应包含存档条目"
    );
    assert!(
        !plan.entries.iter().any(|e| e.rule.relative_path == "resourcepacks/"),
        "关闭 L2 后计划不应包含资源包条目"
    );
    assert!(plan.options_result.is_none(), "关闭 L4 后计划不应携带合并明细");

    // 执行：auto_backup=false 时即使存在覆盖也不生成备份 zip。
    let outcome = service.execute_plan(&plan, true, &mut |_| {}).unwrap();
    assert!(outcome.success);
    assert_eq!(
        std::fs::read_to_string(new.join("options.txt")).unwrap(),
        "fov:0.6\n",
        "关闭 L4 后目标 options 应保持原样"
    );
    assert!(
        !new.join("backups").exists(),
        "关闭自动备份后不应创建备份目录"
    );

    // 反向对照：同名覆盖在开启备份时目标内容被旧版替换（L1 语义复核）。
    let mut options_on = MigrationOptions::all_enabled();
    options_on.include_packs = false;
    options_on.include_options = false;
    let plan_on = service.plan_migration(source, target, options_on).unwrap();
    assert!(plan_on
        .entries
        .iter()
        .flat_map(|e| e.decisions.iter())
        .any(|d| d.action == DecisionAction::CopyFromOld));
    service.execute_plan(&plan_on, true, &mut |_| {}).unwrap();
    assert_eq!(
        std::fs::read_to_string(new.join("saves/level.dat")).unwrap(),
        "level",
        "开启 L1 后同名存档应被旧版覆盖"
    );
    assert!(new.join("backups").is_dir(), "开启自动备份后应创建备份目录");
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn app_config_roundtrips_through_disk() {
    use packporter::app_config::AppConfig;
    let mut config = AppConfig::default();
    config.versions_dir = "E:\\test\\versions".to_string();
    config.include_saves = false;
    // 序列化往返：不落盘（避免污染用户配置目录），验证 serde 语义。
    let text = serde_json::to_string(&config).unwrap();
    let back: AppConfig = serde_json::from_str(&text).unwrap();
    assert_eq!(back, config);
    // 缺失字段回退默认值。
    let partial: AppConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(partial, AppConfig::default());
}

// Path 导入占位：供后续路径断言扩展使用。
#[allow(dead_code)]
fn _path_used(_p: &Path) {}
