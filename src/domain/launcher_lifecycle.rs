//! 文件职责：决定 shim 会话结束时的安全退出，并识别启动器派生进程。
//! 定义范围：纯内存生命周期策略与进程快照匹配，不执行进程枚举或应用启停。

/// 结构职责：决定界面是否需要随最后一个启动器安全退出。
/// 字段说明：session_observed 表示已进入跟随会话，包含启动器已退出但任务仍忙碌的状态。
/// 约束条件：关闭跟随设置清空会话；手动打开的界面必须先观察到启动器才自动退出。
pub struct LauncherWindowLifecycle {
    session_observed: bool,
}

impl LauncherWindowLifecycle {
    /// 函数职责：为手动打开或由启动器唤起的界面建立会话策略。
    /// 输入说明：followed 为 true 表示shim 已建立启动器会话后唤起此界面。
    /// 输出说明：返回独立界面策略，不进行 IO。
    /// 实现思路：跟随启动保留已进入会话的事实，避免启动器在界面就绪前退出而漏关。
    pub fn new(followed: bool) -> Self {
        Self {
            session_observed: followed,
        }
    }

    /// 函数职责：结合设置、启动器快照与任务状态决定是否可以自动退出界面。
    /// 输入说明：enabled 是当前设置，running_count 是成功的完整快照，busy 表示仍有工作不可中断。
    /// 输出说明：只有跟随会话结束且任务空闲时为 true；忙碌期间保留等待，启动器重开则继续会话。
    /// 实现思路：关闭设置清空会话；有启动器时记录会话；无启动器时等待任务空闲。
    pub fn observe(&mut self, enabled: bool, running_count: usize, busy: bool) -> bool {
        if !enabled {
            self.session_observed = false;
            return false;
        }
        if running_count > 0 {
            self.session_observed = true;
            return false;
        }
        self.session_observed && !busy
    }
}

pub use packporter_launcher::process::is_launcher_process;

#[cfg(test)]
mod tests {
    use super::LauncherWindowLifecycle;

    #[test]
    fn manual_window_waits_for_a_launcher_session_before_closing() {
        let mut window = LauncherWindowLifecycle::new(false);
        assert!(!window.observe(true, 0, false));
        assert!(!window.observe(true, 2, false));
        assert!(!window.observe(true, 1, false));
        assert!(window.observe(true, 0, false));
    }

    #[test]
    fn followed_window_waits_until_work_finishes_and_respects_reopened_launcher() {
        let mut window = LauncherWindowLifecycle::new(true);
        assert!(!window.observe(true, 0, true));
        assert!(!window.observe(true, 1, false));
        assert!(!window.observe(true, 0, true));
        assert!(window.observe(true, 0, false));
        assert!(LauncherWindowLifecycle::new(true).observe(true, 0, false));
    }

    #[test]
    fn disabling_follow_cancels_pending_exit_and_requires_a_new_session() {
        let mut window = LauncherWindowLifecycle::new(true);
        assert!(!window.observe(true, 0, true));
        assert!(!window.observe(false, 1, false));
        assert!(!window.observe(true, 0, false));
        assert!(!window.observe(true, 1, false));
        assert!(window.observe(true, 0, false));
    }
}
