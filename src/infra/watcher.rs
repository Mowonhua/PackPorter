//! 文件职责：实现 versions/ 目录监控与"解压完成"稳定性判定（模块 D 默认后端）。
//! 定义范围：NotifyWatcher（notify 封装）、SnapshotProbe（快照比对探针）与目录快照。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::domain::error::{PackError, PackResult};
use crate::services::folder_watcher::StabilityProbe;

// ==================== 常量、枚举和类型别名 ====================

/// 稳定性判定所需的连续一致快照轮数。
pub const STABLE_ROUNDS: usize = 3;

/// 每轮快照间隔：解压类写入的典型停顿窗口。
pub const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(800);

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：一次目录快照：文件集合与总字节数。
 * 字段说明：file_count 与 total_bytes 是稳定性的唯一判据。
 * 约束条件：快照过程只读；目录消失时返回 None。
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirSnapshot {
    /// 目录内文件总数（递归）。
    pub file_count: usize,
    /// 全部文件字节总数。
    pub total_bytes: u64,
}

// ==================== 函数和方法定义 ====================

impl DirSnapshot {
    /**
     * 函数职责：对目录做递归只读快照。
     * 输入说明：dir 为候选版本目录。
     * 输出说明：目录不存在或不可读时返回 None。
     * 实现思路：walkdir 递归统计文件数与字节数。
     */
    pub fn capture(dir: &Path) -> Option<DirSnapshot> {
        if !dir.is_dir() {
            return None;
        }
        let mut file_count = 0usize;
        let mut total_bytes = 0u64;
        for entry in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                file_count += 1;
                total_bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        Some(DirSnapshot { file_count, total_bytes })
    }
}

impl Default for SnapshotProbe {
    /**
     * 函数职责：提供探针默认实例。
     * 输入说明：无。
     * 输出说明：探针实例。
     * 实现思路：无状态类型直接返回单元结构。
     */
    fn default() -> Self {
        Self
    }
}

impl SnapshotProbe {
    /**
     * 函数职责：构造探针。
     * 输入说明：无。
     * 输出说明：探针实例。
     * 实现思路：无状态。
     */
    pub fn new() -> Self {
        Self
    }
}

// ==================== 接口和抽象契约 ====================

/**
 * 结构职责：默认稳定性探针：连续 STABLE_ROUNDS 轮快照一致即判定稳定。
 * 字段说明：无状态，每轮独立比对（快照内容一致即认为写入停顿）。
 * 约束条件：快照间隔由调用方控制（轮询线程使用 SNAPSHOT_INTERVAL）；
 *           目录中途消失视为不稳定并重置连续计数。
 */
pub struct SnapshotProbe;

impl StabilityProbe for SnapshotProbe {
    fn is_stable(&self, dir: &Path) -> bool {
        let mut consecutive = 0usize;
        let mut last: Option<DirSnapshot> = None;
        for _ in 0..STABLE_ROUNDS {
            match DirSnapshot::capture(dir) {
                // 目录消失：写入可能仍在进行（被移动/重命名），判定不稳定。
                None => return false,
                Some(snapshot) => {
                    if last.as_ref() == Some(&snapshot) {
                        consecutive += 1;
                    } else {
                        consecutive = 1;
                    }
                    last = Some(snapshot);
                }
            }
            // 达到连续一致轮数即判定稳定。
            if consecutive >= STABLE_ROUNDS {
                return true;
            }
            std::thread::sleep(SNAPSHOT_INTERVAL);
        }
        false
    }
}

/**
 * 结构职责：notify 库的目录监控封装，负责接收原始文件系统事件。
 * 字段说明：仅过滤"新建目录"类事件，判定逻辑交给 FolderWatcherService。
 * 约束条件：watch 失败返回领域错误；停止后必须释放 watcher。
 */
pub struct NotifyWatcher;

// 全局 watcher 注册表：句柄 → 活跃 watcher（保活）。
type WatcherRegistry = Arc<Mutex<BTreeMap<u64, notify::RecommendedWatcher>>>;

impl NotifyWatcher {
    /**
     * 函数职责：开始监控 versions/ 根目录的新目录事件。
     * 输入说明：root 为 versions/ 目录；callback 在每次相关事件到达时被调用（携带目录名）。
     * 输出说明：成功返回 watcher 句柄；失败返回 FileSystem 错误。
     * 实现思路：notify::recommended_watcher + 非阻塞通道转发，后台线程消费事件并回调。
     */
    pub fn watch_new_dirs(
        root: &Path,
        callback: impl FnMut(String) + Send + 'static,
    ) -> PackResult<u64> {
        // 事件通道：notify 回调线程只做非阻塞投递。
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                // 只关心目录创建/重命名类事件。
                let relevant = matches!(
                    event.kind,
                    notify::EventKind::Create(_) | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                );
                if relevant {
                    for path in event.paths {
                        if path.is_dir() {
                            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                let _ = tx.send(name.to_string());
                            }
                        }
                    }
                }
            }
        })
        .map_err(|e| PackError::FileSystem {
            operation: "watch".to_string(),
            path: root.display().to_string(),
            message: e.to_string(),
        })?;
        // 监控目录本身（非递归：只感知 versions/ 一级变化）。
        use notify::Watcher as _;
        watcher
            .watch(root, notify::RecursiveMode::NonRecursive)
            .map_err(|e: notify::Error| PackError::FileSystem {
                operation: "watch".to_string(),
                path: root.display().to_string(),
                message: e.to_string(),
            })?;

        // 注册表保活 watcher 并分配句柄。
        let handle = next_handle();
        registry().lock().map(|mut reg| reg.insert(handle, watcher)).map_err(|_| {
            PackError::FileSystem {
                operation: "watch".to_string(),
                path: root.display().to_string(),
                message: "watcher 注册表中毒".to_string(),
            }
        })?;

        // 事件消费线程：把目录名转交给 callback。
        std::thread::spawn(move || {
            let mut cb = callback;
            while let Ok(name) = rx.recv() {
                cb(name);
            }
        });
        Ok(handle)
    }

    /**
     * 函数职责：停止并释放指定 watcher。
     * 输入说明：handle 为 watch_new_dirs 返回句柄。
     * 输出说明：幂等。
     * 实现思路：从注册表移除并 drop watcher。
     */
    pub fn unwatch(handle: u64) {
        if let Ok(mut reg) = registry().lock() {
            reg.remove(&handle);
        }
    }
}

/**
 * 函数职责：返回全局 watcher 注册表。
 * 输入说明：无。
 * 输出说明：注册表共享句柄。
 * 实现思路：static Mutex + BTreeMap。
 */
fn registry() -> WatcherRegistry {
    static REGISTRY: std::sync::OnceLock<WatcherRegistry> = std::sync::OnceLock::new();
    REGISTRY
        .get_or_init(|| Arc::new(Mutex::new(BTreeMap::new())))
        .clone()
}

/**
 * 函数职责：生成递增的 watcher 句柄。
 * 输入说明：无。
 * 输出说明：全局递增句柄值。
 * 实现思路：原子计数器。
 */
fn next_handle() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

// Instant 供后续精确停顿判定扩展使用，保留导入。
#[allow(dead_code)]
fn _unused_instant() -> Instant {
    Instant::now()
}
