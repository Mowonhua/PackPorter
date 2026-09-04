//! 文件职责：实现 Zip 镜像备份的打包与还原原语，供 BackupEngine 复用。
//! 定义范围：zip 打包函数、还原函数与备份命名契约；不含事务编排。

use std::io::Read;
use std::path::Path;

use crate::domain::error::{PackError, PackResult};
use crate::domain::transaction::RollbackReport;

// ==================== 函数和方法定义 ====================

/**
 * 函数职责：将指定文件集合打包为 zip 镜像，zip 内保留以 root 为基准的相对路径。
 * 输入说明：files 为将被覆盖的既有文件绝对路径列表；root 为相对路径基准目录；
 *           dest_zip 为输出 zip 绝对路径；progress 上报逐文件进度。
 * 输出说明：成功返回打包文件数；任一文件读取失败即整体失败返回 Backup 错误。
 * 实现思路：zip::ZipWriter 逐条 start_file + copy_file_io，压缩方式 deflate。
 */
pub fn pack_files(
    files: &[std::path::PathBuf],
    root: &Path,
    dest_zip: &Path,
    progress: &mut dyn FnMut(usize, usize),
) -> PackResult<usize> {
    // 输出 zip 的父目录必须存在。
    if let Some(parent) = dest_zip.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            PackError::Backup(format!("创建备份目录失败 [{}]: {e}", parent.display()))
        })?;
    }
    let zip_file = std::fs::File::create(dest_zip)
        .map_err(|e| PackError::Backup(format!("创建备份文件失败: {e}")))?;
    let mut writer = zip::ZipWriter::new(zip_file);
    // 时间戳选项：使用 zip 库支持的简单文件选项，deflate 压缩。
    let options: zip::write::SimpleFileOptions =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let total = files.len();
    let mut packed = 0usize;
    for (index, file) in files.iter().enumerate() {
        // zip 内路径：以 root 为基准的相对路径（统一 '/' 分隔）。
        let relative = file
            .strip_prefix(root)
            .map_err(|_| PackError::Backup(format!("文件不在基准目录内: {}", file.display())))?
            .to_string_lossy()
            .replace('\\', "/");
        writer
            .start_file(relative, options)
            .map_err(|e| PackError::Backup(format!("写入 zip 条目失败: {e}")))?;
        let mut f = std::fs::File::open(file)
            .map_err(|e| PackError::Backup(format!("读取待备份文件失败 [{}]: {e}", file.display())))?;
        std::io::copy(&mut f, &mut writer)
            .map_err(|e| PackError::Backup(format!("压缩写入失败: {e}")))?;
        packed += 1;
        progress(index + 1, total);
    }
    writer
        .finish()
        .map_err(|e| PackError::Backup(format!("收尾 zip 失败: {e}")))?;
    Ok(packed)
}

/**
 * 函数职责：将 zip 镜像按内部记录的相对路径还原到 root 目录下。
 * 输入说明：zip_path 为备份产物；root 为还原基准目录。
 * 输出说明：还原报告，含失败明细；zip 缺失或损坏返回 Backup 错误。
 * 实现思路：逐条目读取并在 root 下重建父目录后覆盖写入；单条失败记入报告继续。
 */
pub fn unpack_to(zip_path: &Path, root: &Path) -> PackResult<RollbackReport> {
    let file = std::fs::File::open(zip_path)
        .map_err(|e| PackError::Backup(format!("打开备份文件失败: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| PackError::Backup(format!("备份 zip 损坏: {e}")))?;
    let mut report = RollbackReport::default();
    for index in 0..archive.len() {
        let mut entry = match archive.by_index(index) {
            Ok(entry) => entry,
            Err(e) => {
                report.failed += 1;
                report
                    .log
                    .push_str(&format!("条目 {index} 读取失败: {e}\n"));
                continue;
            }
        };
        // 还原目标：root + zip 内相对路径。
        let Some(relative) = entry.enclosed_name() else {
            report.failed += 1;
            report.log.push_str("条目含非法路径，已跳过\n");
            continue;
        };
        let target = root.join(relative);
        if entry.is_dir() {
            let _ = std::fs::create_dir_all(&target);
            continue;
        }
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut content = Vec::new();
        if entry.read_to_end(&mut content).is_err()
            || std::fs::write(&target, content).is_err()
        {
            report.failed += 1;
            report
                .log
                .push_str(&format!("还原失败: {}\n", target.display()));
        } else {
            report.restored += 1;
        }
    }
    Ok(report)
}

/**
 * 函数职责：生成备份文件名（时间戳 + 固定前缀）。
 * 输入说明：now 为当前时刻。
 * 输出说明：形如 "20260212-153001-pre-migrate.zip" 的文件名。
 * 实现思路：chrono 本地时间格式化。
 */
pub fn backup_file_name(now: chrono::DateTime<chrono::Local>) -> String {
    now.format("%Y%m%d-%H%M%S-pre-migrate.zip").to_string()
}
