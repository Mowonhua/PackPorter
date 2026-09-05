use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use packporter::domain::instance::MigrationOptions;
use packporter::services::backup_engine::BackupEngine;
use packporter::services::migration_service::MigrationService;

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "packporter_backup_scope_{}_{}",
            std::process::id(),
            id
        ));
        std::fs::create_dir_all(root.join("Old")).unwrap();
        std::fs::create_dir_all(root.join("New")).unwrap();
        Self(root)
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn plan(root: &Path) -> packporter::domain::instance::MigrationPlan {
    let service = MigrationService::new(root.to_path_buf());
    let profiles = service.instances.scan_instances().unwrap();
    let old = profiles
        .iter()
        .find(|p| p.version.dir_name == "Old")
        .unwrap();
    let new = profiles
        .iter()
        .find(|p| p.version.dir_name == "New")
        .unwrap();
    service
        .plan_migration(old, new, MigrationOptions::all_enabled())
        .unwrap()
}

#[test]
fn keep_new_only_skips_archive_creation() {
    let fixture = Fixture::new();
    fixture.write("Old/resourcepacks/shared.zip", "old pack");
    fixture.write("New/resourcepacks/shared.zip", "new pack");
    let plan = plan(&fixture.0);
    let engine = BackupEngine::for_instance(plan.target.root_dir.clone());
    let archive = engine.backup_before(&plan, &mut |_| {}).unwrap();
    assert!(
        archive.as_os_str().is_empty(),
        "保留新版的资源不需要迁移前备份"
    );
    assert!(!engine.backup_root.exists());
}

#[test]
fn backup_contains_only_overwritten_content_and_options() {
    let fixture = Fixture::new();
    for (relative, content) in [
        ("Old/resourcepacks/shared.zip", "old pack"),
        ("New/resourcepacks/shared.zip", "new pack"),
        ("Old/saves/level.dat", "old world"),
        ("New/saves/level.dat", "new world"),
        ("Old/saves/new.dat", "new file"),
        ("Old/options.txt", "fov:0.85\n"),
        ("New/options.txt", "fov:0.6\n"),
    ] {
        fixture.write(relative, content);
    }
    let plan = plan(&fixture.0);
    let engine = BackupEngine::for_instance(plan.target.root_dir.clone());
    let archive = engine.backup_before(&plan, &mut |_| {}).unwrap();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(archive).unwrap()).unwrap();
    let mut names = archive.file_names().map(str::to_owned).collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["options.txt", "saves/level.dat"]);
    for (name, expected) in [
        ("options.txt", "fov:0.6\n"),
        ("saves/level.dat", "new world"),
    ] {
        let mut actual = String::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_string(&mut actual)
            .unwrap();
        assert_eq!(actual, expected);
    }
}
