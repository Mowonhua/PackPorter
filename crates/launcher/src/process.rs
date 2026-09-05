//! 文件职责：识别受控会话中的启动器进程。
//! 定义范围：根据可执行名称和 Java 参数判断入口；不执行 IO，也不决定窗口生命周期。

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
    use super::is_launcher_process;

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
