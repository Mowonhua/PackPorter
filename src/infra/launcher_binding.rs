//! 文件职责：维护原路径 shim、原始启动器备份及受管清单。
//! 定义范围：安装记录、可信入口读取与可逆文件事务。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::{fs, io::Write};

/// 结构职责：持久化一个受管启动入口及文件身份。
/// 字段说明：所有路径均为绝对路径；摘要为 SHA-256 小写十六进制。
/// 约束条件：schema 固定为 1，backup 必须是 launcher 的同目录 .bak.exe 文件。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Binding {
    pub schema: u32,
    pub launcher: PathBuf,
    pub backup: PathBuf,
    pub app: PathBuf,
    pub original_sha256: String,
    pub shim_sha256: String,
}

/// 函数职责：将设置中的入口集合应用到当前用户的受管启动器。
/// 输入说明：关闭时忽略 launchers 并恢复全部清单条目。
/// 输出说明：失败时返回包含补偿失败信息的中文错误。
/// 实现思路：定位主程序、独立 shim 和清单目录后交给文件事务入口。
pub fn apply(enabled: bool, launchers: &[String]) -> Result<(), String> {
    let app = std::env::current_exe().map_err(|e| e.to_string())?;
    let shim = app.with_file_name("packporter-shim.exe");
    let config = crate::app_config::AppConfig::config_path().ok_or("无法定位用户配置目录")?;
    apply_at(
        enabled,
        &launchers.iter().map(PathBuf::from).collect::<Vec<_>>(),
        &app,
        &shim,
        config.parent().ok_or("配置路径缺少父目录")?,
    )
}

/// 函数职责：在指定文件位置安装或还原受管入口。
/// 输入说明：app 与 shim 必须是文件，启用时只接受普通 exe 入口。
/// 输出说明：先验证全部目标，任何步骤失败时逆序补偿已完成步骤。
/// 实现思路：校验文件身份，生成可逆改动，最后发布完整清单。
pub fn apply_at(
    enabled: bool,
    launchers: &[PathBuf],
    app: &Path,
    shim: &Path,
    registry_dir: &Path,
) -> Result<(), String> {
    fs::create_dir_all(registry_dir).map_err(|e| format!("无法创建受管清单目录：{e}"))?;
    // 锁由文件句柄持有，进程退出自动释放；保留空锁文件避免删除后产生两把独立锁。
    let transaction_lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(registry_dir.join("launcher-bindings.lock"))
        .map_err(|e| format!("无法打开启动器事务锁：{e}"))?;
    transaction_lock
        .try_lock()
        .map_err(|e| format!("另一项启动器关联操作尚未结束：{e}"))?;
    let registry = registry_dir.join("launcher-bindings.json");
    let old_bytes = optional_bytes(&registry)?;
    let previous: Vec<Binding> = match &old_bytes {
        Some(bytes) => {
            serde_json::from_slice(bytes).map_err(|e| format!("受管启动器清单损坏：{e}"))?
        }
        None => Vec::new(),
    };
    let mut known = std::collections::HashSet::new();
    for binding in &previous {
        if !known.insert(binding.launcher.clone()) {
            return Err("受管启动器清单存在重复入口".into());
        }
        if read_binding(&binding.launcher)?.as_ref() != Some(binding) {
            return Err(format!(
                "入口旁车与受管清单不符：{}",
                binding.launcher.display()
            ));
        }
    }
    let mut desired = Vec::new();
    let mut changes = Vec::new();
    if enabled && !launchers.is_empty() {
        let app = canonical_file(app)?;
        let shim = canonical_file(shim)?;
        let shim_bytes = fs::read(&shim).map_err(|e| e.to_string())?;
        let shim_hash = digest(&shim_bytes);
        let mut selected = std::collections::HashSet::new();
        for path in launchers {
            let launcher = canonical_file(path)?;
            if !selected.insert(launcher.clone()) {
                continue;
            }
            let backup = backup_path(&launcher)?;
            if launcher == app || launcher == shim || backup == app || backup == shim {
                return Err("不能将 PackPorter 或 shim 自身作为启动器关联".into());
            }
            if let Some(binding) = previous.iter().find(|b| b.launcher == launcher) {
                desired.push(binding.clone());
                continue;
            }
            if optional_bytes(&backup)?.is_some()
                || optional_bytes(&sidecar_path(&launcher))?.is_some()
            {
                return Err(format!(
                    "备份或旁车已存在且不受当前清单管理，请先处理：{}",
                    backup.display()
                ));
            }
            let original = fs::read(&launcher).map_err(|e| e.to_string())?;
            if digest(&original) == shim_hash {
                return Err("不能关联另一个 shim 副本".into());
            }
            let binding = Binding {
                schema: 1,
                launcher: launcher.clone(),
                backup: backup.clone(),
                app: app.clone(),
                original_sha256: digest(&original),
                shim_sha256: shim_hash.clone(),
            };
            // 先保留原文件，再发布旁车，最后接管原入口；shim 从不会看到尚不存在的备份。
            changes.push(Change {
                path: backup,
                source: None,
                before: None,
                after: Some(original.clone()),
            });
            changes.push(Change {
                path: sidecar_path(&launcher),
                source: None,
                before: None,
                after: Some(serde_json::to_vec_pretty(&binding).map_err(|e| e.to_string())?),
            });
            changes.push(Change {
                path: launcher,
                source: None,
                before: Some(original),
                after: Some(shim_bytes.clone()),
            });
            desired.push(binding);
        }
    }
    for binding in &previous {
        if desired.iter().any(|b| b.launcher == binding.launcher) {
            continue;
        }
        // 移动原始文件保留运行中 EXE 的身份，恢复入口后才移除旁车。
        let mut restore = Change::capture(&binding.launcher, optional_bytes(&binding.backup)?)?;
        restore.source = Some(binding.backup.clone());
        changes.push(restore);
        changes.push(Change::capture(&sidecar_path(&binding.launcher), None)?);
    }
    let next_bytes = serde_json::to_vec_pretty(&desired).map_err(|e| e.to_string())?;
    if changes.is_empty()
        && (old_bytes.as_ref() == Some(&next_bytes) || (old_bytes.is_none() && desired.is_empty()))
    {
        return Ok(());
    }
    changes.push(Change {
        path: registry,
        source: None,
        before: old_bytes,
        after: Some(next_bytes),
    });
    fs::create_dir_all(registry_dir).map_err(|e| format!("无法创建受管清单目录：{e}"))?;
    execute_changes(&changes)
}

/// 函数职责：读取并验证入口旁的受管记录。
/// 输入说明：executable 为实际启动的 shim 绝对路径。
/// 输出说明：无旁车返回 None；损坏记录、摘要或路径不符返回错误。
/// 实现思路：校验旁车结构、备份身份与当前 shim 身份，防止递归或误执行。
pub fn read_binding(executable: &Path) -> Result<Option<Binding>, String> {
    let executable = canonical_file(executable)?;
    let Some(bytes) = optional_bytes(&sidecar_path(&executable))? else {
        return Ok(None);
    };
    let binding: Binding =
        serde_json::from_slice(&bytes).map_err(|e| format!("启动器关联旁车损坏：{e}"))?;
    if binding.schema != 1
        || binding.launcher != executable
        || binding.backup != backup_path(&executable)?
        || !binding.app.is_absolute()
        || binding.app == executable
        || binding.app == binding.backup
        || binding
            .app
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("packporter-shim.exe"))
        || binding.app.with_file_name("packporter-shim.exe") == executable
    {
        return Err("启动器关联记录的版本或路径不合法".into());
    }
    if binding.app.is_file()
        && digest(&fs::read(&binding.app).map_err(|e| e.to_string())?) == binding.shim_sha256
    {
        return Err("主应用路径指向 shim，已拒绝递归启动".into());
    }
    if digest(&fs::read(&executable).map_err(|e| e.to_string())?) != binding.shim_sha256 {
        return Err(format!(
            "启动器入口已被更新或替换，为防止旧备份覆盖新版，未执行恢复；请保留并处理备份：{}",
            executable.display()
        ));
    }
    let original = fs::read(&binding.backup)
        .map_err(|e| format!("无法读取启动器备份 {}：{e}", binding.backup.display()))?;
    if digest(&original) != binding.original_sha256
        || binding.original_sha256 == binding.shim_sha256
    {
        return Err("启动器备份身份不符或指向 shim，已拒绝继续".into());
    }
    Ok(Some(binding))
}

fn canonical_file(path: &Path) -> Result<PathBuf, String> {
    if !path.is_file() {
        return Err(format!("不是可用文件：{}", path.display()));
    }
    fs::canonicalize(path).map_err(|e| format!("无法解析文件路径 {}：{e}", path.display()))
}

fn backup_path(launcher: &Path) -> Result<PathBuf, String> {
    let stem = launcher.file_stem().ok_or("启动器缺少文件名")?;
    if launcher
        .extension()
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("exe"))
        || stem
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".bak")
    {
        return Err("只支持 .exe 启动器，不能选择 .bak.exe 备份".into());
    }
    let mut name = stem.to_os_string();
    name.push(".bak.exe");
    Ok(launcher.with_file_name(name))
}

fn sidecar_path(launcher: &Path) -> PathBuf {
    let mut value = launcher.as_os_str().to_os_string();
    value.push(".packporter.json");
    PathBuf::from(value)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn optional_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("无法读取 {}：{e}", path.display())),
    }
}

/// 每项改动保留修改前字节；预检覆盖完整目标集合，清单始终最后发布。
struct Change {
    /// 恢复使用同目录重命名，避免删除运行中被映射的备份 EXE。
    source: Option<PathBuf>,
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
}
impl Change {
    fn capture(path: &Path, after: Option<Vec<u8>>) -> Result<Self, String> {
        Ok(Self {
            path: path.to_owned(),
            source: None,
            before: optional_bytes(path)?,
            after,
        })
    }
}

fn execute_changes(changes: &[Change]) -> Result<(), String> {
    for (index, change) in changes.iter().enumerate() {
        // 在每次写入前再次比较内容，避免覆盖预检之后发生的启动器更新。
        let actual = optional_bytes(&change.path);
        let result = match actual {
            Ok(bytes) if bytes == change.before => match &change.source {
                Some(source) => move_original(source, change),
                None => replace_bytes(&change.path, change.after.as_deref()),
            },
            Ok(_) => Err((
                format!("文件在安装期间变化：{}", change.path.display()),
                false,
            )),
            Err(error) => Err((error, false)),
        };
        if let Err((error, touched)) = result {
            let mut errors = vec![error];
            let end = index + usize::from(touched);
            for completed in changes[..end].iter().rev() {
                if let Err(rollback_error) = rollback_change(completed) {
                    errors.push(format!("回滚失败：{rollback_error}"));
                }
            }
            return Err(errors.join("；"));
        }
    }
    Ok(())
}

fn move_original(source: &Path, change: &Change) -> Result<(), (String, bool)> {
    if optional_bytes(source).map_err(|e| (e, false))? != change.after {
        return Err((format!("备份在恢复期间变化：{}", source.display()), false));
    }
    fs::remove_file(&change.path).map_err(|e| {
        (
            format!("无法移除受管入口 {}：{e}", change.path.display()),
            false,
        )
    })?;
    rename_no_replace(source, &change.path)
        .map_err(|e| (format!("无法还原启动器 {}：{e}", source.display()), true))
}

fn rollback_change(change: &Change) -> Result<(), String> {
    let actual = optional_bytes(&change.path)?;
    if actual.is_some() && actual != change.after {
        return Err(format!(
            "文件已被外部修改，未覆盖：{}",
            change.path.display()
        ));
    }
    if let Some(source) = &change.source {
        if actual == change.after {
            if source.exists() {
                return Err(format!("备份路径被占用，未覆盖：{}", source.display()));
            }
            rename_no_replace(&change.path, source)
                .map_err(|e| format!("无法撤销启动器还原：{e}"))?;
        }
    }
    replace_bytes(&change.path, change.before.as_deref()).map_err(|(e, _)| e)
}

/// 完整写入同目录临时文件后才发布，避免启动时读取半个 EXE。
/// 删除失败表示原文件仍在；删除之后发布失败必须补偿，不终止持有入口的进程。
fn replace_bytes(path: &Path, bytes: Option<&[u8]>) -> Result<(), (String, bool)> {
    let prepared = bytes
        .map(|bytes| prepare_file(path, bytes))
        .transpose()
        .map_err(|error| (error, false))?;
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            let mut error = format!("无法替换 {}（请退出关联启动器后重试）：{e}", path.display());
            if let Some(temporary) = &prepared {
                if let Err(cleanup) = fs::remove_file(temporary) {
                    error.push_str(&format!(
                        "；无法清理临时文件 {}：{cleanup}",
                        temporary.display()
                    ));
                }
            }
            return Err((error, false));
        }
    }
    if let Some(temporary) = prepared {
        if let Err(e) = rename_no_replace(&temporary, path) {
            let mut error = format!("无法发布 {}：{e}", path.display());
            if let Err(cleanup) = fs::remove_file(&temporary) {
                error.push_str(&format!(
                    "；无法清理临时文件 {}：{cleanup}",
                    temporary.display()
                ));
            }
            return Err((error, true));
        }
    }
    Ok(())
}

fn prepare_file(path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(
        ".packporter-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = PathBuf::from(name);
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|e| format!("无法创建临时文件 {}：{e}", temporary.display()))?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let mut error = format!("无法准备文件 {}：{e}", path.display());
        if let Err(cleanup) = fs::remove_file(&temporary) {
            error.push_str(&format!(
                "；无法清理临时文件 {}：{cleanup}",
                temporary.display()
            ));
        }
        return Err(error);
    }
    Ok(temporary)
}

/// 发布时不得覆盖预检后由外部创建的入口；Windows 默认 MoveFileExW 不替换已有目标。
fn rename_no_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        // 字符缓冲区在调用期间有效；flags=0 明确禁止覆盖已有目标。
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                0,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(source, target)?;
        fs::remove_file(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture {
        dir: PathBuf,
        app: PathBuf,
        shim: PathBuf,
        launcher: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "packporter-binding-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            let app = dir.join("packporter.exe");
            let shim = dir.join("packporter-shim.exe");
            let launcher = dir.join("PCL2.exe");
            fs::write(&app, b"app").unwrap();
            fs::write(&shim, b"shim").unwrap();
            fs::write(&launcher, b"original\0\xff").unwrap();
            Self {
                dir,
                app,
                shim,
                launcher,
            }
        }
        fn apply(&self, enabled: bool, paths: &[PathBuf]) -> Result<(), String> {
            apply_at(enabled, paths, &self.app, &self.shim, &self.dir)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.dir).unwrap();
        }
    }
    #[test]
    fn failed_later_write_rolls_back_completed_restore() {
        let f = Fixture::new();
        f.apply(true, std::slice::from_ref(&f.launcher)).unwrap();
        let binding = read_binding(&f.launcher).unwrap().unwrap();
        let mut restore =
            Change::capture(&binding.launcher, optional_bytes(&binding.backup).unwrap()).unwrap();
        restore.source = Some(binding.backup.clone());
        let failure = Change {
            source: None,
            path: f.dir.join("missing-parent").join("record.json"),
            before: None,
            after: Some(b"record".to_vec()),
        };
        assert!(execute_changes(&[restore, failure]).is_err());
        assert!(read_binding(&f.launcher).unwrap().is_some());
        assert_eq!(fs::read(&binding.backup).unwrap(), b"original\0\xff");
    }
    #[test]
    fn removed_entry_is_restored_and_missing_app_does_not_prevent_disable() {
        let f = Fixture::new();
        let second = f.dir.join("HMCL.exe");
        fs::write(&second, b"hmcl").unwrap();
        f.apply(true, &[f.launcher.clone(), second.clone()])
            .unwrap();
        f.apply(true, std::slice::from_ref(&second)).unwrap();
        assert_eq!(fs::read(&f.launcher).unwrap(), b"original\0\xff");
        assert!(!f.dir.join("PCL2.bak.exe").exists());
        assert!(read_binding(&second).unwrap().is_some());
        fs::remove_file(&f.app).unwrap();
        fs::remove_file(&f.shim).unwrap();
        assert!(read_binding(&second).unwrap().is_some());
        f.apply(false, &[]).unwrap();
        assert_eq!(fs::read(&second).unwrap(), b"hmcl");
    }
    #[test]
    fn rollback_does_not_overwrite_external_changes() {
        let f = Fixture::new();
        let change = Change::capture(&f.launcher, Some(b"ours".to_vec())).unwrap();
        fs::write(&f.launcher, b"external").unwrap();
        assert!(rollback_change(&change).is_err());
        assert_eq!(fs::read(&f.launcher).unwrap(), b"external");
    }
    #[test]
    fn original_bytes_are_restored_on_disable() {
        let f = Fixture::new();
        f.apply(true, std::slice::from_ref(&f.launcher)).unwrap();
        assert_eq!(
            fs::read(f.dir.join("PCL2.bak.exe")).unwrap(),
            b"original\0\xff"
        );
        assert_eq!(fs::read(&f.launcher).unwrap(), b"shim");
        assert!(read_binding(&f.launcher).unwrap().is_some());
        f.apply(false, &[]).unwrap();
        assert_eq!(fs::read(&f.launcher).unwrap(), b"original\0\xff");
        assert!(!f.dir.join("PCL2.bak.exe").exists());
        assert!(read_binding(&f.launcher).unwrap().is_none());
    }
    #[test]
    fn preflight_failure_does_not_modify_any_launcher() {
        let f = Fixture::new();
        let second = f.dir.join("HMCL.exe");
        fs::write(&second, b"hmcl").unwrap();
        fs::write(f.dir.join("HMCL.bak.exe"), b"occupied").unwrap();
        assert!(f
            .apply(true, &[f.launcher.clone(), second.clone()])
            .is_err());
        assert_eq!(fs::read(&f.launcher).unwrap(), b"original\0\xff");
        assert_eq!(fs::read(second).unwrap(), b"hmcl");
        assert_eq!(fs::read(f.dir.join("HMCL.bak.exe")).unwrap(), b"occupied");
        assert!(!f.dir.join("PCL2.bak.exe").exists());
    }
    #[test]
    fn repeated_install_is_idempotent_and_changed_launcher_is_not_overwritten() {
        let f = Fixture::new();
        f.apply(true, &[f.launcher.clone(), f.launcher.clone()])
            .unwrap();
        f.apply(true, std::slice::from_ref(&f.launcher)).unwrap();
        fs::write(&f.launcher, b"updated-launcher").unwrap();
        assert!(f.apply(false, &[]).is_err());
        assert_eq!(fs::read(&f.launcher).unwrap(), b"updated-launcher");
        assert_eq!(
            fs::read(f.dir.join("PCL2.bak.exe")).unwrap(),
            b"original\0\xff"
        );
    }
}
