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

/// 函数职责：根据可执行文件名称与 Java 启动入口识别 PCL/PCL2 或 HMCL。
/// 输入说明：name 是进程名称；cmd 是包含可执行文件的操作系统参数列表，未拼接为命令字符串。
/// 输出说明：只有明确的启动器名称或 Java 入口匹配才为 true，游戏参数中的启动器名称不匹配。
/// 实现思路：先判断本机启动器名称，再检查 Java 的 -jar 目标或主类，主入口之后不再搜索。
pub fn is_launcher_process(name: &str, cmd: &[String]) -> bool {
    let name = name.to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "pcl.exe" | "pcl2.exe" | "plain craft launcher 2.exe" | "hmcl.exe"
    ) {
        return true;
    }
    if name.strip_prefix("hmcl-").is_some_and(|suffix| {
        suffix.starts_with(|character: char| character.is_ascii_digit()) && suffix.ends_with(".exe")
    }) {
        return true;
    }
    if !matches!(name.as_str(), "java" | "javaw" | "java.exe" | "javaw.exe") {
        return false;
    }

    let mut arguments = cmd.iter().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "-jar" {
            return arguments.next().is_some_and(|target| {
                // 同时接受 Windows 与 Unix 路径，保证纯函数与运行测试的平台无关。
                let filename = target
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                filename.starts_with("hmcl") && filename.ends_with(".jar")
            });
        }
        // 这些 Java 选项独占后续参数，类路径中的同名字符串不是主入口。
        if matches!(
            argument.as_str(),
            "-cp"
                | "-classpath"
                | "--class-path"
                | "-p"
                | "--module-path"
                | "--upgrade-module-path"
                | "--add-modules"
                | "--limit-modules"
                | "--add-exports"
                | "--add-opens"
                | "--add-reads"
                | "--patch-module"
        ) {
            arguments.next();
            continue;
        }
        if argument.starts_with('-') {
            continue;
        }
        // 主类之后的参数属于应用，不能继续搜索，否则会把 Minecraft 误识别为启动器。
        return argument == "org.jackhuang.hmcl.Launcher";
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{is_launcher_process, LauncherWindowLifecycle};

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

    #[test]
    fn recognizes_supported_launcher_executables() {
        for name in [
            "PCL.exe",
            "pcl2.EXE",
            "Plain Craft Launcher 2.exe",
            "HMCL.exe",
            "HMCL-3.6.11.exe",
        ] {
            assert!(is_launcher_process(name, &[]), "{name}");
        }
        for name in [
            "minecraft.exe",
            "pcl-helper.exe",
            "hmcl-helper.exe",
            "javaw.exe",
        ] {
            assert!(!is_launcher_process(name, &[]), "{name}");
        }
    }

    #[test]
    fn recognizes_java_jar_and_class_entry_points() {
        for args in [
            vec![
                "javaw.exe",
                "-Xmx512m",
                "-jar",
                r"C:\Launchers\HMCL-3.6.11.jar",
            ],
            vec!["java", "-jar", "/opt/hmcl.jar"],
            vec!["javaw.exe", "-cp", "libs/*", "org.jackhuang.hmcl.Launcher"],
        ] {
            let cmd: Vec<String> = args.into_iter().map(String::from).collect();
            assert!(is_launcher_process(&cmd[0], &cmd), "{cmd:?}");
        }
    }

    #[test]
    fn ignores_hmcl_names_in_game_arguments_and_java_option_values() {
        for args in [
            vec![
                "javaw.exe",
                "net.minecraft.client.main.Main",
                "--version",
                "HMCL-3.6.11",
            ],
            vec![
                "javaw.exe",
                "-cp",
                "org.jackhuang.hmcl.Launcher",
                "net.minecraft.client.main.Main",
            ],
            vec!["javaw.exe", "-jar", "minecraft.jar", "-jar", "HMCL.jar"],
            vec![
                "javaw.exe",
                "-Dlauncher=HMCL.jar",
                "net.minecraft.client.main.Main",
            ],
            vec!["other.exe", "-jar", "HMCL.jar"],
            vec!["javaw.exe", "-jar"],
        ] {
            let cmd: Vec<String> = args.into_iter().map(String::from).collect();
            assert!(!is_launcher_process(&cmd[0], &cmd), "{cmd:?}");
        }
    }
}
