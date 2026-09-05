//! 文件职责：模块 C —— 轻量 Zip 镜像备份与类事务执行（失败自动回滚）。
//! 定义范围：BackupEngine 结构、备份/还原/列出实现与 MigrationTransaction 实现。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::domain::error::{PackError, PackResult};
use crate::domain::instance::{DecisionAction, MigrationPlan, MigrationProgress};
use crate::domain::transaction::{
    MigrationTransaction, RollbackReport, TransactionAction, UndoAction,
};
use crate::infra::zip_archive;

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：备份与事务引擎的配置载体。
 * 字段说明：backup_root 为全部备份的根目录，默认在目标实例内 backups/ 下。
 * 约束条件：backup_root 必须可写；Zip 创建失败时中止迁移，避免无备份覆盖。
 */
#[derive(Clone)]
pub struct BackupEngine {
    /// 备份根目录绝对路径。
    pub backup_root: PathBuf,
}

// ==================== 函数和方法定义 ====================

impl BackupEngine {
    /**
     * 函数职责：以目标实例路径构造引擎，备份根定为 <实例>/backups。
     * 输入说明：target_instance_dir 为新版实例目录。
     * 输出说明：始终成功。
     * 实现思路：拼接实例目录与 "backups" 子目录。
     */
    pub fn for_instance(target_instance_dir: PathBuf) -> Self {
        Self {
            backup_root: target_instance_dir.join("backups"),
        }
    }

    /**
     * 函数职责：对目标实例中被迁移涉及的文件做增量 Zip 镜像备份。
     * 输入说明：plan 用于收集将被覆盖的既有文件；progress 用于上报打包进度。
     * 输出说明：返回本次备份 zip 的绝对路径；无可备份文件时返回空路径。
     *           备份目录不可写时返回 Backup 错误。
     * 实现思路：收集 plan 中覆盖写动作的目标既有文件 → 打包为
     *           backups/<时间戳>-pre-migrate.zip；全部为新建文件时跳过打包。
     */
    pub fn backup_before(
        &self,
        plan: &MigrationPlan,
        progress: &mut dyn FnMut(MigrationProgress),
    ) -> PackResult<PathBuf> {
        // 与 to_actions 的复制条件保持一致：KeepNew 和 SourceMissing 不会写入目标，
        // 不应读取或压缩这些文件，尤其是通常体积较大的同名资源包和光影包。
        let mut seen = HashSet::new();
        let mut targets = plan
            .entries
            .iter()
            .flat_map(|entry| entry.decisions.iter())
            .filter(|decision| decision.action == DecisionAction::CopyFromOld)
            .map(|decision| plan.target.root_dir.join(&decision.relative_path))
            .filter(|path| seen.insert(path.clone()) && path.is_file())
            .collect::<Vec<_>>();

        // L4 合并同样覆盖写入既有偏好文件，必须纳入备份范围（路径来自计划规则）。
        for outcome in &plan.options_results {
            let merged_path = plan.target.root_dir.join(&outcome.relative_path);
            if seen.insert(merged_path.clone()) && merged_path.is_file() {
                targets.push(merged_path);
            }
        }

        // 全部为新建（新实例无既有文件）时无需备份。
        if targets.is_empty() {
            return Ok(PathBuf::new());
        }

        let zip_path = self
            .backup_root
            .join(zip_archive::backup_file_name(chrono::Local::now()));
        // 适配 zip 打包回调签名 (done, total) 到统一进度事件。
        let mut report_progress = |done: usize, total_items: usize| {
            progress(MigrationProgress {
                done,
                total: total_items,
                current: "备份中".to_string(),
            });
        };
        zip_archive::pack_files(&targets, &plan.target.root_dir, &zip_path, &mut report_progress)?;
        Ok(zip_path)
    }

    /**
     * 函数职责：从指定 zip 镜像还原全部文件（回滚的兜底路径）。
     * 输入说明：backup_zip 为 backup_before 产物。
     * 输出说明：返回还原报告；zip 缺失或损坏时返回 Backup 错误。
     * 实现思路：委托 zip_archive::unpack_to，还原基准为目标实例根目录。
     */
    pub fn restore_from_zip(&self, backup_zip: &Path) -> PackResult<RollbackReport> {
        let root = self
            .backup_root
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        zip_archive::unpack_to(backup_zip, &root)
    }

    /**
     * 函数职责：列出当前实例已有备份及其体积，供 UI 展示与清理。
     * 输入说明：无。
     * 输出说明：(zip 路径, 字节数) 列表，按时间倒序（文件名含时间戳，倒序即最新在前）。
     * 实现思路：扫描 backup_root 下 *.zip 并读取元数据，按文件名倒序。
     */
    pub fn list_backups(&self) -> PackResult<Vec<(PathBuf, u64)>> {
        let mut result = Vec::new();
        if !self.backup_root.is_dir() {
            return Ok(result);
        }
        for entry in std::fs::read_dir(&self.backup_root)
            .map_err(|e| PackError::FileSystem {
                operation: "read".to_string(),
                path: self.backup_root.display().to_string(),
                message: e.to_string(),
            })?
            .flatten()
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("zip") {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                result.push((path, size));
            }
        }
        result.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(result)
    }
}

// ==================== 接口和抽象契约 ====================

impl MigrationTransaction for BackupEngine {
    /**
     * 函数职责：以事务语义执行动作清单：任一失败即逆序回滚全部已完成动作。
     * 输入说明：actions 为计划动作；progress 上报每步进度。
     * 输出说明：全部成功返回应用动作数；失败返回 RolledBack 并附回滚报告。
     * 实现思路：执行前对将被覆盖的既有目标做内存快照 → 逐动作执行并登记补偿 →
     *           失败时逆序执行补偿（Delete 新建项 / Restore 覆盖项），恢复到迁移前状态。
     */
    fn execute(
        &self,
        actions: &[TransactionAction],
        progress: &mut dyn FnMut(MigrationProgress),
    ) -> PackResult<usize> {
        let total = actions.len();
        let mut applied: Vec<UndoAction> = Vec::new();
        let mut report = |done: usize, current: &str| {
            progress(MigrationProgress {
                done,
                total,
                current: current.to_string(),
            });
        };

        for (index, action) in actions.iter().enumerate() {
            let result = match action {
                TransactionAction::CopyFile { source, destination } => {
                    copy_file_tracked(source, destination, &mut applied)
                }
                TransactionAction::WriteText { destination, content } => {
                    write_text_tracked(destination, content, &mut applied)
                }
            };
            report(index + 1, &describe(action));
            // 任一动作失败：立即逆序回滚并报告。
            if let Err(fail_reason) = result {
                let rollback = rollback_applied(&applied);
                return Err(PackError::RolledBack {
                    reason: fail_reason,
                    report: format!(
                        "已回滚 {} 项（成功 {}，失败 {}）\n{}",
                        rollback.restored + rollback.failed,
                        rollback.restored,
                        rollback.failed,
                        rollback.log
                    ),
                });
            }
        }
        Ok(applied.len())
    }
}

/**
 * 函数职责：生成动作的用户可读描述，用于进度展示。
 * 输入说明：action 为事务动作。
 * 输出说明：简短中文描述。
 * 实现思路：按动作类型拼接目标路径。
 */
fn describe(action: &TransactionAction) -> String {
    match action {
        TransactionAction::CopyFile { destination, .. } => {
            format!("复制 {}", destination.display())
        }
        TransactionAction::WriteText { destination, .. } => {
            format!("合并写入 {}", destination.display())
        }
    }
}

/**
 * 函数职责：执行单文件复制并登记补偿动作。
 * 输入说明：source/destination 为绝对路径；applied 为补偿动作累积列表（可变借用）。
 * 输出说明：成功返回 Ok(())；失败返回中文错误描述，且此时可能已登记补偿。
 * 实现思路：创建父目录 → 若目标已存在先登记 Restore 快照 → 复制 → 登记 Delete 补偿。
 */
fn copy_file_tracked(
    source: &Path,
    destination: &Path,
    applied: &mut Vec<UndoAction>,
) -> Result<(), String> {
    // 确保目标父目录存在。
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 [{}]: {e}", parent.display()))?;
    }
    // 目标已存在：先快照原内容，供回滚还原（覆盖语义）。
    let existed = destination.exists();
    if existed {
        let content = std::fs::read(destination)
            .map_err(|e| format!("快照读取失败 [{}]: {e}", destination.display()))?;
        applied.push(UndoAction::Restore {
            original_path: destination.to_path_buf(),
            content,
        });
    }
    std::fs::copy(source, destination)
        .map_err(|e| format!("复制失败 [{} -> {}]: {e}", source.display(), destination.display()))?;
    if !existed {
        // 新建文件：回滚时直接删除。
        applied.push(UndoAction::Delete {
            path: destination.to_path_buf(),
        });
    }
    Ok(())
}

/**
 * 函数职责：执行文本覆盖写入并登记补偿动作。
 * 输入说明：destination 为目标文件；content 为完整新文本；applied 为补偿累积列表。
 * 输出说明：成功返回 Ok(())；失败返回中文错误描述。
 * 实现思路：与 copy_file_tracked 相同的快照-写入-登记流程。
 */
fn write_text_tracked(
    destination: &Path,
    content: &str,
    applied: &mut Vec<UndoAction>,
) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 [{}]: {e}", parent.display()))?;
    }
    let existed = destination.exists();
    if existed {
        let snapshot = std::fs::read(destination)
            .map_err(|e| format!("快照读取失败 [{}]: {e}", destination.display()))?;
        applied.push(UndoAction::Restore {
            original_path: destination.to_path_buf(),
            content: snapshot,
        });
    }
    std::fs::write(destination, content)
        .map_err(|e| format!("写入失败 [{}]: {e}", destination.display()))?;
    if !existed {
        applied.push(UndoAction::Delete {
            path: destination.to_path_buf(),
        });
    }
    Ok(())
}

/**
 * 函数职责：逆序执行全部已登记的补偿动作，恢复迁移前状态。
 * 输入说明：applied 为按执行顺序登记的补偿动作。
 * 输出说明：回滚报告；单条补偿失败不中断整体回滚。
 * 实现思路：逆序遍历：Restore 写回快照字节；Delete 删除新建文件（含空父目录清理）。
 */
fn rollback_applied(applied: &[UndoAction]) -> RollbackReport {
    let mut report = RollbackReport::default();
    for action in applied.iter().rev() {
        let result = match action {
            UndoAction::Restore { original_path, content } => std::fs::write(original_path, content)
                .map_err(|e| format!("还原失败 [{}]: {e}", original_path.display())),
            UndoAction::Delete { path } => std::fs::remove_file(path)
                .map_err(|e| format!("删除失败 [{}]: {e}", path.display()))
                // 新建文件删除后，顺带清理因迁移新建的空父目录（尽力而为）。
                .and_then(|_| cleanup_empty_parents(path)),
        };
        match result {
            Ok(()) => report.restored += 1,
            Err(message) => {
                report.failed += 1;
                report.log.push_str(&message);
                report.log.push('\n');
            }
        }
    }
    report
}

/**
 * 函数职责：自底向上清理因迁移新建的空目录（到实例根目录为止）。
 * 输入说明：file_path 为刚删除的文件路径。
 * 输出说明：始终 Ok；目录非空或删除失败时静默停止。
 * 实现思路：逐级上溯 parent，目录为空则删除，非空即停。
 */
fn cleanup_empty_parents(file_path: &Path) -> Result<(), String> {
    let mut current = file_path.parent();
    while let Some(dir) = current {
        // 目录非空或删除失败时停止上溯。
        if std::fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(true) {
            break;
        }
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
        current = dir.parent();
    }
    Ok(())
}
