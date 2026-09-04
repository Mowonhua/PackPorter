//! 文件职责：实现 java 进程枚举与实例目录占用判定（模块 A 的占用检测后端）。
//! 定义范围：ProcessProbe 结构与占用判定实现；sysinfo 依赖仅出现在本模块。

use std::path::Path;

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：单个候选进程的摘要信息。
 * 字段说明：cmdline 截断存放，仅用于与实例路径做包含匹配。
 * 约束条件：只读快照，不持有进程句柄。
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    /// 系统 PID。
    pub pid: u32,
    /// 进程名，如 javaw.exe。
    pub name: String,
    /// 启动命令行（可截断），用于路径关联判定。
    pub cmdline: String,
}

// ==================== 接口和抽象契约 ====================

/**
 * 结构职责：进程探测门面，供 InstanceService 调用。
 * 字段说明：无内部状态；每次调用实时刷新进程表。
 * 约束条件：不得 panic；sysinfo 初始化失败按无进程处理。
 */
pub struct ProcessProbe;

impl ProcessProbe {
    /**
     * 函数职责：枚举系统中全部 java 系进程（java/javaw 及其 .exe 变体）。
     * 输入说明：无。
     * 输出说明：进程摘要列表；枚举失败返回空列表（不阻断主流程）。
     * 实现思路：sysinfo 刷新进程表，按进程名小写前缀过滤。
     */
    pub fn list_java_processes(&self) -> Vec<ProcessInfo> {
        let mut sys = sysinfo::System::new();
        // 只刷新进程信息，避免全量系统扫描开销。
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let mut result = Vec::new();
        for (pid, process) in sysinfo::System::processes(&sys) {
            let name = process.name().to_string_lossy().to_lowercase();
            if !name.starts_with("java") {
                continue;
            }
            let cmdline = process
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(" ");
            result.push(ProcessInfo {
                pid: pid.as_u32(),
                name: process.name().to_string_lossy().to_string(),
                cmdline,
            });
        }
        result
    }

    /**
     * 函数职责：判定任一 java 进程是否与实例目录存在关联占用。
     * 输入说明：instance_dir 为实例目录绝对路径。
     * 输出说明：占用时返回进程摘要；未占用返回 None。
     * 实现思路：命令行包含实例目录字符串即判定占用（统一小写后比较，
     *           兼容 Windows 大小写不敏感路径）。
     */
    pub fn find_locker(&self, instance_dir: &Path) -> Option<ProcessInfo> {
        let dir_lower = instance_dir.to_string_lossy().to_lowercase();
        self.list_java_processes()
            .into_iter()
            .find(|p| p.cmdline.to_lowercase().contains(&dir_lower))
    }
}
