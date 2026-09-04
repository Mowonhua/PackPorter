//! 文件职责：模块 A —— 扫描 versions/ 目录，产出实例画像并检测进程占用。
//! 定义范围：InstanceService 的结构与扫描/占用检测实现。

use std::path::PathBuf;

use crate::domain::error::{PackError, PackResult};
use crate::domain::instance::{InstanceProfile, MinecraftVersion, VersionProfileReader};
use crate::infra::process_probe::ProcessProbe;

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：实例探测服务的运行时依赖集合。
 * 字段说明：profile_reader 决定版本元数据解析策略，默认实现为 JsonProfileReader。
 * 约束条件：versions_root 必须存在；服务本身无状态，可并发复用。
 */
#[derive(Clone)]
pub struct InstanceService {
    /// versions/ 目录绝对路径（如 E:\...\.minecraft\versions）。
    pub versions_root: PathBuf,
    /// 版本 profile 解析器抽象。
    pub profile_reader: std::sync::Arc<dyn VersionProfileReader>,
}

// ==================== 函数和方法定义 ====================

impl InstanceService {
    /**
     * 函数职责：构造指向指定 .minecraft/versions 的探测服务。
     * 输入说明：versions_root 为 versions 目录绝对路径。
     * 输出说明：始终成功返回服务实例（可用性在扫描时校验）。
     * 实现思路：打包路径与默认 profile 读取器。
     */
    pub fn new(versions_root: PathBuf) -> Self {
        Self {
            versions_root,
            profile_reader: default_profile_reader(),
        }
    }

    /**
     * 函数职责：扫描 versions/ 下全部版本目录，产出每个实例的画像。
     * 输入说明：无（使用自身 versions_root）。
     * 输出说明：按目录名排序的画像列表；根目录不可用时返回 PathUnavailable。
     * 实现思路：枚举一级子目录，跳过无 jar/json 的空目录，逐个调用 profile_reader.read，
     *           单个实例解析失败不阻断整体扫描（跳过并继续）。
     */
    pub fn scan_instances(&self) -> PackResult<Vec<InstanceProfile>> {
        let mut profiles = Vec::new();
        for version in self.list_versions()? {
            // 单实例解析失败仅跳过该实例，不中断全量扫描。
            if let Ok(profile) = self.profile_reader.read(&self.versions_root.join(&version.dir_name), &version.dir_name) {
                profiles.push(profile);
            }
        }
        Ok(profiles)
    }

    /**
     * 函数职责：列出 versions/ 下全部版本目录标识（轻量探测，不解析 json）。
     * 输入说明：无。
     * 输出说明：目录名与关联 jar 名的列表，按名称排序；根目录不可用时报错。
     * 实现思路：读取一级子目录，匹配 <dirname>.jar；目录名排序保证 UI 稳定展示。
     */
    pub fn list_versions(&self) -> PackResult<Vec<MinecraftVersion>> {
        let entries = std::fs::read_dir(&self.versions_root).map_err(|_| {
            PackError::PathUnavailable(self.versions_root.display().to_string())
        })?;
        let mut versions = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            // 只处理一级子目录；文件（如临时压缩包）不是实例。
            if !path.is_dir() {
                continue;
            }
            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // 关联 jar：优先 <目录名>.jar；缺失时尝试目录内第一个 .jar 的文件名。
            let jar_name = if path.join(format!("{dir_name}.jar")).exists() {
                dir_name.to_string()
            } else {
                std::fs::read_dir(&path)
                    .map(|entries| {
                        entries
                            .flatten()
                            .map(|f| f.path())
                            .find(|p| p.extension().and_then(|e| e.to_str()) == Some("jar"))
                            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
                            .unwrap_or_default()
                    })
                    .unwrap_or_default()
            };
            versions.push(MinecraftVersion {
                dir_name: dir_name.to_string(),
                jar_name,
            });
        }
        versions.sort_by(|a, b| a.dir_name.cmp(&b.dir_name));
        Ok(versions)
    }

    /**
     * 函数职责：检测是否有运行中的 java 进程占用了指定实例目录。
     * 输入说明：profile 为待检查实例画像。
     * 输出说明：被占用返回 InstanceLocked；未占用返回 Ok(())。
     * 实现思路：枚举 java 系进程，命令行包含实例目录路径即判定占用
     *           （Windows 路径大小写不敏感，统一小写后比较）。
     */
    pub fn ensure_unlocked(&self, profile: &InstanceProfile) -> PackResult<()> {
        let probe = ProcessProbe;
        let dir_lower = profile.root_dir.to_string_lossy().to_lowercase();
        if let Some(locker) = probe
            .list_java_processes()
            .into_iter()
            .find(|p| p.cmdline.to_lowercase().contains(&dir_lower))
        {
            return Err(PackError::InstanceLocked {
                instance_name: profile.version.dir_name.clone(),
                pid: locker.pid,
                process_name: locker.name,
            });
        }
        Ok(())
    }
}

/**
 * 函数职责：提供默认的 JSON 版本 profile 读取器。
 * 输入说明：无。
 * 输出说明：返回位于基础设施层的 JsonProfileReader 实例。
 * 实现思路：直接构造 infra 层类型并装箱为 trait 对象。
 */
pub fn default_profile_reader() -> std::sync::Arc<dyn VersionProfileReader> {
    std::sync::Arc::new(crate::infra::json_profile::JsonProfileReader)
}
