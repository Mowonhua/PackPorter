/**
 * 文件职责：真实环境冒烟示例：对指定 versions 目录做只读扫描与 options 合并预览。
 * 定义范围：example 二进制；仅调用库公开 API，不写任何文件。
 */

use packporter::services::instance_service::InstanceService;
use packporter::services::migration_service::MigrationService;
use std::path::PathBuf;

fn main() {
    // 目标 versions 目录来自命令行参数，缺省指向参考环境。
    let versions_root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"E:\Minecraft\Minecraft-PCL2\.minecraft\versions"));

    println!("== 扫描 {} ==", versions_root.display());
    let service = InstanceService::new(versions_root.clone());
    let profiles = match service.scan_instances() {
        Ok(profiles) => profiles,
        Err(e) => {
            eprintln!("扫描失败: {e}");
            return;
        }
    };
    for p in &profiles {
        println!(
            "  [{}] mc={} loader={:?} {} jar={}",
            p.version.dir_name,
            p.mc_version,
            p.loader,
            p.loader_version.as_deref().unwrap_or("-"),
            p.version.jar_name
        );
    }

    // 以 NAST hard 新旧版本对做 L4 合并预览（只读）。
    let old = profiles
        .iter()
        .find(|p| p.version.dir_name.contains("0.9.3"))
        .cloned();
    let new = profiles
        .iter()
        .find(|p| p.version.dir_name.contains("0.9.6"))
        .cloned();
    if let (Some(old), Some(new)) = (old, new) {
        println!("\n== L4 options 合并预览: {} -> {} ==", old.version.dir_name, new.version.dir_name);
        let migration = MigrationService::new(versions_root);
        match migration.plan_migration(&old, &new) {
            Ok(plan) => {
                for entry in &plan.entries {
                    println!(
                        "  {:?} {}: {} 项",
                        entry.rule.level, entry.rule.relative_path, entry.total_items
                    );
                }
                if let Some(result) = &plan.options_result {
                    println!(
                        "  options 合并摘要: {}",
                        packporter::services::options_merge::summarize(&result.outcomes)
                    );
                    let preview = migration.preview_options(&plan).unwrap_or_default();
                    let lines = preview.lines().count();
                    println!("  合并后 options 共 {lines} 行（预览前 8 行）:");
                    for line in preview.lines().take(8) {
                        println!("    {line}");
                    }
                }
            }
            Err(e) => eprintln!("计划生成失败: {e}"),
        }
    } else {
        println!("\n未找到 NAST hard 新旧版本对，跳过合并预览。");
    }
}
