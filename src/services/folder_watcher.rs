//! 文件职责：模块 D —— 监控 versions/ 目录，感知新实例目录解压完成并回调 UI。
//! 定义范围：FolderWatcherService 结构、事件模型、启停实现与事件投递。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use crate::domain::error::{PackError, PackResult};
use crate::infra::watcher::{NotifyWatcher, SnapshotProbe};

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：目录监控事件，描述"哪个新版本目录已就绪"。
 * 字段说明：dir_name 为 versions/ 下新出现的目录名。
 * 约束条件：只有当目录判定为"完整解压"后才允许产生事件；中途目录不触发。
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceArrivalEvent {
    /// 新就绪的版本目录名。
    pub dir_name: String,
    /// versions/ 目录的绝对路径。
    pub versions_root: PathBuf,
    /// 事件产生时刻（本地时区）。
    pub detected_at: chrono::DateTime<chrono::Local>,
}

// ==================== 接口和抽象契约 ====================

/**
 * 接口职责：抽象"目录是否已稳定"的判定策略，使等待逻辑与文件系统解耦。
 * 调用方：FolderWatcherService 依赖它过滤抖动；测试可注入假实现模拟解压过程。
 * 实现要求：判定必须容忍拷贝过程中的临时文件与写入停顿；不得修改任何文件。
 */
pub trait StabilityProbe: Send + Sync {
    /**
     * 函数职责：判断指定目录当前是否稳定（写入停顿、句柄释放）。
     * 输入说明：dir 为候选版本目录绝对路径。
     * 输出说明：true 表示可视为完整解压。
     * 实现思路：对比前后两次扫描的文件集合与总字节数，连续多轮一致即判定稳定。
     */
    fn is_stable(&self, dir: &Path) -> bool;
}

/**
 * 结构职责：目录监控服务的运行时状态与依赖。
 * 字段说明：sender 供监控线程向 UI 事件循环投递事件；stop_flag 驱动线程退出；
 *           known_dirs 记录启动时已存在的目录，只有新增目录才触发事件。
 * 约束条件：同一服务实例同时只允许一个活跃 watch 会话；重复 start 应返回错误。
 */
pub struct FolderWatcherService {
    /// 被监控的 versions/ 目录。
    pub versions_root: PathBuf,
    /// 稳定性判定策略。
    pub probe: Arc<dyn StabilityProbe>,
    /// 事件投递通道。
    pub sender: mpsc::Sender<InstanceArrivalEvent>,
    /// 停止标记。
    pub stop_flag: Arc<AtomicBool>,
    /// 启动时已存在的目录名集合（跨线程共享给监控线程）。
    known_dirs: Arc<Mutex<std::collections::BTreeSet<String>>>,
    /// 当前活跃会话句柄；None 表示未启动。
    active_handle: Option<u64>,
}

// ==================== 函数和方法定义 ====================

impl FolderWatcherService {
    /**
     * 函数职责：构造监控服务并返回事件接收端。
     * 输入说明：versions_root 为监控目标；probe 为稳定性策略。
     * 输出说明：(服务, 事件接收器)；UI 在事件循环中非阻塞接收。
     * 实现思路：建立无界 mpsc 通道，服务持有发送端，并以目录扫描初始化 known_dirs。
     */
    pub fn new(
        versions_root: PathBuf,
        probe: Arc<dyn StabilityProbe>,
    ) -> (Self, mpsc::Receiver<InstanceArrivalEvent>) {
        let (tx, rx) = mpsc::channel();
        // 启动时已存在的目录不触发"新实例"事件。
        let known: std::collections::BTreeSet<String> = std::fs::read_dir(&versions_root)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        (
            Self {
                versions_root,
                probe,
                sender: tx,
                stop_flag: Arc::new(AtomicBool::new(false)),
                known_dirs: Arc::new(Mutex::new(known)),
                active_handle: None,
            },
            rx,
        )
    }

    /**
     * 函数职责：启动后台监控线程，发现新目录且稳定后投递 InstanceArrivalEvent。
     * 输入说明：无。
     * 输出说明：成功返回监控句柄 id；已有活跃会话时返回 InvalidPlan 错误。
     * 实现思路：notify 监听目录创建/重命名事件 → 捕获新目录名 → 后台线程轮询
     *           probe.is_stable 直至稳定或 stop → 通过 sender 投递事件；stop_flag 置位退出。
     */
    pub fn start(&mut self) -> PackResult<u64> {
        // 单会话约束：已有活跃监控时拒绝重复启动。
        if self.active_handle.is_some() {
            return Err(PackError::InvalidPlan("监控会话已在运行".to_string()));
        }
        let handle = next_handle();
        let root = self.versions_root.clone();
        let probe = self.probe.clone();
        let sender = self.sender.clone();
        let stop_flag = self.stop_flag.clone();
        let known_dirs = self.known_dirs.clone();

        // 原始事件接收：新目录名先进入待确认集合。
        let pending: Arc<Mutex<std::collections::BTreeSet<String>>> =
            Arc::new(Mutex::new(std::collections::BTreeSet::new()));
        let pending_for_callback = pending.clone();
        let root_for_callback = root.clone();

        NotifyWatcher::watch_new_dirs(&root, move |dir_name| {
            // 只处理首次出现的新目录。
            let is_new = known_dirs
                .lock()
                .map(|mut set| set.insert(dir_name.clone()))
                .unwrap_or(false);
            if is_new {
                let _ = pending_for_callback
                    .lock()
                    .map(|mut set| set.insert(dir_name.clone()));
                let _ = &root_for_callback;
            }
        })?;

        // 稳定性轮询线程：周期检查 pending 集合，稳定即投递事件。
        let poll_probe = probe;
        let poll_root = root;
        std::thread::spawn(move || {
            let snapshot_probe = SnapshotProbe::new();
            let _ = &snapshot_probe;
            loop {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                // 取出待确认目录逐个检查稳定性。
                let candidates: Vec<String> = pending
                    .lock()
                    .map(|set| set.iter().cloned().collect())
                    .unwrap_or_default();
                for dir_name in candidates {
                    let dir = poll_root.join(&dir_name);
                    if dir.is_dir() && poll_probe.is_stable(&dir) {
                        // 稳定即投递并移出待确认集合。
                        if pending.lock().map(|mut set| set.remove(&dir_name)).unwrap_or(false) {
                            let _ = sender.send(InstanceArrivalEvent {
                                dir_name,
                                versions_root: poll_root.clone(),
                                detected_at: chrono::Local::now(),
                            });
                        }
                    }
                }
                std::thread::sleep(crate::infra::watcher::SNAPSHOT_INTERVAL);
            }
        });

        self.active_handle = Some(handle);
        Ok(handle)
    }

    /**
     * 函数职责：停止监控线程并释放 watcher 资源。
     * 输入说明：handle 为 start 返回的句柄。
     * 输出说明：幂等；重复停止安全。
     * 实现思路：置位 stop_flag、释放 watcher、清空活跃句柄。
     */
    pub fn stop(&mut self, handle: u64) {
        if self.active_handle != Some(handle) {
            return;
        }
        self.stop_flag.store(true, Ordering::Relaxed);
        NotifyWatcher::unwatch(handle);
        self.active_handle = None;
    }
}

// ==================== 常量、枚举和类型别名 ====================

/**
 * 函数职责：生成递增的监控会话句柄。
 * 输入说明：无。
 * 输出说明：全局递增句柄值。
 * 实现思路：原子计数器。
 */
fn next_handle() -> u64 {
    use std::sync::atomic::AtomicU64;
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}
