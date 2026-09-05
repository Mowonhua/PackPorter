//! 文件职责：独立启动入口与受控启动器会话追踪。
//! 定义范围：启动参数、命名 Job 会话及进程存活查询。
use std::{ffi::OsString, path::PathBuf};

/// 结构职责：包装入口的启动请求。
/// 字段说明：launcher 为 EXE 或 JAR，java 仅用于 JAR；arguments 原样转交启动器。
/// app 指定联动应用位置，省略时使用当前 shim 同目录下的 packporter.exe。
/// 约束条件：只启动指定路径，不更改绑定文件；进程退出不得终止启动的游戏。
pub struct ShimLaunch {
    pub launcher: PathBuf,
    pub java: Option<PathBuf>,
    pub app: Option<PathBuf>,
    pub arguments: Vec<OsString>,
}

/// 文件身份校验保留规范路径；传给外部启动器时改用普通盘符或 UNC 路径。
/// .NET Framework 启动器的 WPF 初始化会拒绝扩展路径前缀，即使 CreateProcessW 能创建进程。
/// UTF-16 转换保留路径字符；没有普通路径等价形式的设备路径不作猜测。
fn compatible_launch_path(path: &std::path::Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let raw: Vec<u16> = path.as_os_str().encode_wide().collect();
        const SEPARATOR: u16 = b'\\' as u16;
        if raw.starts_with(&[SEPARATOR, SEPARATOR, b'?' as u16, SEPARATOR]) {
            if raw.len() >= 7
                && raw[5] == b':' as u16
                && raw[6] == SEPARATOR
                && ((65..=90).contains(&raw[4]) || (97..=122).contains(&raw[4]))
            {
                return PathBuf::from(OsString::from_wide(&raw[4..]));
            }
            if raw.len() >= 8
                && raw[7] == SEPARATOR
                && raw[4..7]
                    .iter()
                    .zip(b"unc")
                    .all(|(value, expected)| (*value | 32) == u16::from(*expected))
            {
                let ordinary: Vec<u16> = [SEPARATOR, SEPARATOR]
                    .into_iter()
                    .chain(raw[8..].iter().copied())
                    .collect();
                return PathBuf::from(OsString::from_wide(&ordinary));
            }
        }
    }
    path.to_owned()
}

/// 将已通过 read_binding 验证的入口交给中央 shim，立即释放原路径 EXE 的运行锁。
/// 原入口不得持有整个启动器会话，否则关闭设置时 Windows 无法删除并还原该文件。
/// 中央程序缺失或不可用时仍启动已验证的备份；此时不提供 PackPorter 联动。
pub fn handoff(binding: crate::binding::Binding, arguments: Vec<OsString>) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::process::Command;
    let central = binding.app.with_file_name("packporter-shim.exe");
    let directory =
        compatible_launch_path(binding.launcher.parent().ok_or("启动器路径没有父目录")?);
    // 摘要验证防止中央路径误指启动器或另一份受管入口，避免转交形成递归。
    let central_valid = binding.app.is_file()
        && central != binding.launcher
        && std::fs::read(&central)
            .is_ok_and(|bytes| format!("{:x}", Sha256::digest(bytes)) == binding.shim_sha256)
        && matches!(crate::binding::read_binding(&central), Ok(None));
    if central_valid {
        let mut command = Command::new(central);
        command
            .arg("--launcher")
            .arg(&binding.backup)
            .arg("--app")
            .arg(&binding.app)
            .arg("--")
            .args(&arguments)
            .current_dir(&directory);
        if command.spawn().is_ok() {
            return Ok(());
        }
    }
    Command::new(compatible_launch_path(&binding.backup))
        .args(arguments)
        .current_dir(directory)
        .spawn()
        .map_err(|error| format!("无法启动原启动器备份：{error}"))?;
    Err("原启动器已启动，但 PackPorter 或中央 shim 不可用；请恢复程序文件后重新关联".into())
}

/// 启动启动器；启用联动时持有其会话至最后一个启动器进程退出。
/// 失败不会终止已经恢复运行的启动器，也不会终止 Minecraft。
pub fn run(launch: ShimLaunch) -> Result<(), String> {
    #[cfg(windows)]
    {
        platform::run(launch)
    }
    #[cfg(not(windows))]
    {
        let _ = launch;
        Err("启动器 shim 目前仅支持 Windows".into())
    }
}

/// 只统计 shim 登记的会话；查询失败必须向 UI 传播，不能据此触发关闭。
pub fn launcher_count() -> Result<usize, String> {
    #[cfg(windows)]
    {
        platform::launcher_count()
    }
    #[cfg(not(windows))]
    {
        Ok(0)
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::{process::is_launcher_process, settings};
    use serde::{Deserialize, Serialize};
    use std::{
        ffi::OsStr,
        fs,
        os::windows::{ffi::OsStrExt, process::CommandExt},
        path::Path,
        process::Command,
        ptr::{null, null_mut},
        time::Duration,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, HANDLE},
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicProcessIdList,
                OpenJobObjectW, QueryInformationJobObject,
            },
            Threading::{
                CreateProcessW, ResumeThread, TerminateProcess, CREATE_NO_WINDOW, CREATE_SUSPENDED,
                PROCESS_INFORMATION, STARTUPINFOW,
            },
        },
    };

    /// 句柄只负责释放引用；Job 不设置 KILL_ON_JOB_CLOSE，游戏可独立继续运行。
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// root_pid 必须同时出现在 Job 成员中才有效，避免 PID 复用导致错误跟随。
    #[derive(Serialize, Deserialize)]
    struct Session {
        job_name: String,
        root_pid: u32,
    }
    struct SessionFile(PathBuf);
    impl Drop for SessionFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    /// 使用 Windows CRT 参数规则：引号前的反斜杠加倍，尾部反斜杠在闭引号前加倍。
    /// 全程保留 UTF-16，避免将合法 Windows 路径通过 UTF-8 有损转换。
    fn quote(argument: &OsStr) -> Vec<u16> {
        let mut output = vec![b'"' as u16];
        let mut slashes = 0;
        for value in argument.encode_wide() {
            if value == b'\\' as u16 {
                slashes += 1;
                continue;
            }
            if value == b'"' as u16 {
                output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
            } else {
                output.extend(std::iter::repeat_n(b'\\' as u16, slashes));
            }
            slashes = 0;
            output.push(value);
        }
        output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        output.push(b'"' as u16);
        output
    }

    fn command_line(executable: &OsStr, arguments: &[OsString]) -> Result<Vec<u16>, String> {
        let mut output = quote(executable);
        for argument in arguments {
            output.push(b' ' as u16);
            output.extend(quote(argument));
        }
        if output.contains(&0) || output.len() >= 32767 {
            return Err("启动参数含 NUL 或超过 Windows 长度限制".into());
        }
        output.push(0);
        Ok(output)
    }

    fn sessions_dir() -> Result<PathBuf, String> {
        settings::config_path()
            .and_then(|path| path.parent().map(|parent| parent.join("launcher-sessions")))
            .ok_or_else(|| "无法定位启动器会话目录".into())
    }

    fn register(directory: &Path, session: &Session) -> Result<SessionFile, String> {
        fs::create_dir_all(directory)
            .map_err(|error| format!("无法创建启动器会话目录：{error}"))?;
        let path = directory.join(format!("{}.json", std::process::id()));
        let temporary = path.with_extension("tmp");
        let bytes = serde_json::to_vec(session).map_err(|error| error.to_string())?;
        fs::write(&temporary, bytes)
            .and_then(|()| fs::rename(&temporary, &path))
            .map_err(|error| format!("无法登记启动器会话：{error}"))?;
        Ok(SessionFile(path))
    }

    /// 以 usize 对齐的缓冲区接收可变长成员数组；MORE_DATA 时重试，不能截断成零。
    fn members(job: &Handle) -> Result<Vec<u32>, String> {
        let mut capacity = 64usize;
        loop {
            let bytes = 8 + capacity * std::mem::size_of::<usize>();
            let mut buffer = vec![0usize; bytes.div_ceil(std::mem::size_of::<usize>())];
            let success = unsafe {
                QueryInformationJobObject(
                    job.0,
                    JobObjectBasicProcessIdList,
                    buffer.as_mut_ptr().cast(),
                    bytes as u32,
                    null_mut(),
                )
            };
            if success == 0 {
                if unsafe { GetLastError() } == ERROR_MORE_DATA && capacity < 1_048_576 {
                    capacity *= 2;
                    continue;
                }
                return Err(format!(
                    "无法查询启动器会话：{}",
                    std::io::Error::last_os_error()
                ));
            }
            let pointer = buffer.as_ptr().cast::<u8>();
            let count = unsafe { *pointer.add(4).cast::<u32>() } as usize;
            if count > capacity {
                return Err("启动器会话成员数量无效".into());
            }
            return Ok((0..count)
                .map(|index| unsafe {
                    *pointer
                        .add(8 + index * std::mem::size_of::<usize>())
                        .cast::<usize>() as u32
                })
                .collect());
        }
    }

    /// 先固定 Job 成员，再只刷新这些 PID；不得先取全局快照，否则 root 退出并
    /// 派生 Java 的交接瞬间会把新子进程误判为不存在。未知成员留待下次观察。
    fn active(job: &Handle, session: &Session) -> Result<usize, String> {
        let members = members(job)?;
        if members.contains(&session.root_pid) {
            return Ok(1);
        }
        if members.is_empty() {
            return Ok(0);
        }
        let pids: Vec<sysinfo::Pid> = members.into_iter().map(sysinfo::Pid::from_u32).collect();
        let mut system = sysinfo::System::new();
        // sysinfo 的默认进程刷新不包含命令行；必须显式读取，才能区分 HMCL 与游戏 JVM。
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&pids),
            true,
            sysinfo::ProcessRefreshKind::nothing().with_cmd(sysinfo::UpdateKind::Always),
        );
        let mut count = 0;
        for pid in pids {
            let process = system
                .process(pid)
                .ok_or("启动器会话成员已变化，等待重新查询")?;
            let name = process.name().to_string_lossy();
            if name.is_empty() {
                return Err("无法读取会话成员进程名".into());
            }
            let command: Vec<String> = process
                .cmd()
                .iter()
                .map(|value| value.to_string_lossy().into_owned())
                .collect();
            if matches!(
                name.to_ascii_lowercase().as_str(),
                "java" | "javaw" | "java.exe" | "javaw.exe"
            ) && command.is_empty()
            {
                return Err("无法读取会话内 Java 入口，等待重新查询".into());
            }
            if is_launcher_process(&name, &command) {
                count += 1;
            }
        }
        Ok(count)
    }
    pub(super) fn launcher_count() -> Result<usize, String> {
        let directory = sessions_dir()?;
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(format!("无法读取启动器会话：{error}")),
        };
        let mut count = 0;
        for entry in entries {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.extension() != Some(OsStr::new("json")) {
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("无法读取启动器会话：{error}")),
            };
            let session: Session = serde_json::from_slice(&bytes)
                .map_err(|error| format!("启动器会话损坏：{error}"))?;
            if !session.job_name.starts_with("Local\\PackPorter.Launcher.") {
                return Err("启动器会话名称无效".into());
            }
            let name = wide(OsStr::new(&session.job_name));
            // JOB_OBJECT_QUERY = 0x0004；仅查询，不授予终止或修改会话的权限。
            let raw = unsafe { OpenJobObjectW(0x0004, 0, name.as_ptr()) };
            if raw.is_null() {
                if unsafe { GetLastError() } == ERROR_FILE_NOT_FOUND {
                    let _ = fs::remove_file(path);
                    continue;
                }
                return Err(format!(
                    "无法打开启动器会话：{}",
                    std::io::Error::last_os_error()
                ));
            }
            let running = active(&Handle(raw), &session)?;
            count += running;
            if running == 0 {
                let _ = fs::remove_file(path);
            }
        }
        Ok(count)
    }

    pub(super) fn run(launch: ShimLaunch) -> Result<(), String> {
        let launcher = compatible_launch_path(
            &fs::canonicalize(&launch.launcher)
                .map_err(|error| format!("无法定位启动器：{error}"))?,
        );
        let jar = launcher
            .extension()
            .is_some_and(|value| value.eq_ignore_ascii_case("jar"));
        if !jar && launch.java.is_some() {
            return Err("--java 仅可用于 JAR 启动器".into());
        }
        let mut arguments = launch.arguments;
        let executable = if jar {
            arguments.insert(0, launcher.as_os_str().to_owned());
            arguments.insert(0, OsString::from("-jar"));
            compatible_launch_path(&launch.java.unwrap_or_else(|| PathBuf::from("javaw.exe")))
        } else {
            launcher.clone()
        };
        let mut command = command_line(executable.as_os_str(), &arguments)?;
        let directory = wide(launcher.parent().ok_or("启动器路径没有父目录")?.as_os_str());
        let enabled = settings::follow_launchers();
        let app = match launch.app {
            Some(app) => app,
            None => std::env::current_exe()
                .map_err(|error| error.to_string())?
                .with_file_name("packporter.exe"),
        };
        if enabled && !app.is_file() {
            return Err("请将 packporter-shim.exe 与 packporter.exe 放在同一目录".into());
        }
        let session = Session {
            job_name: format!(
                "Local\\PackPorter.Launcher.{}.{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|error| error.to_string())?
                    .as_nanos()
            ),
            root_pid: 0,
        };
        let job_name = wide(OsStr::new(&session.job_name));
        let job = Handle(unsafe { CreateJobObjectW(null(), job_name.as_ptr()) });
        if job.0.is_null() {
            return Err(format!(
                "无法创建启动器会话：{}",
                std::io::Error::last_os_error()
            ));
        }
        let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // 必须先暂停再加入 Job，避免启动器先派生 Java 后父进程退出而丢失会话。
        let created = unsafe {
            CreateProcessW(
                null(),
                command.as_mut_ptr(),
                null(),
                null(),
                0,
                CREATE_SUSPENDED,
                null(),
                directory.as_ptr(),
                &startup,
                &mut process,
            )
        };
        if created == 0 {
            return Err(format!(
                "无法启动启动器：{}",
                std::io::Error::last_os_error()
            ));
        }
        let process_handle = Handle(process.hProcess);
        let thread_handle = Handle(process.hThread);
        if unsafe { AssignProcessToJobObject(job.0, process_handle.0) } == 0 {
            let error = std::io::Error::last_os_error();
            // 此时入口线程尚未运行，没有游戏或用户任务；失败必须回收暂停的子进程。
            unsafe {
                TerminateProcess(process_handle.0, 1);
            }
            return Err(format!("无法将启动器加入会话：{error}"));
        }
        if unsafe { ResumeThread(thread_handle.0) } == u32::MAX {
            let error = std::io::Error::last_os_error();
            unsafe {
                TerminateProcess(process_handle.0, 1);
            }
            return Err(format!("无法恢复启动器：{error}"));
        }
        if !enabled {
            return Ok(());
        }
        let session = Session {
            root_pid: process.dwProcessId,
            ..session
        };
        let _registration = register(&sessions_dir()?, &session)?;
        Command::new(app)
            .arg("--launcher-follow")
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|error| format!("启动器已启动，但无法打开 PackPorter：{error}"))?;
        loop {
            // 关闭联动只撤销会话，不结束启动器。查询暂时失败时保留会话，避免误关 UI。
            if !settings::follow_launchers() {
                break;
            }
            if matches!(active(&job, &session), Ok(0)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn launch_paths_preserve_drive_unc_and_device_identity() {
            for (source, expected) in [
                (
                    r"\\?\E:\启动器 目录\PCL.bak.exe",
                    r"E:\启动器 目录\PCL.bak.exe",
                ),
                (r"\\?\UNC\server\share\HMCL.exe", r"\\server\share\HMCL.exe"),
                (r"\\?\Volume{test}\app.exe", r"\\?\Volume{test}\app.exe"),
                (r"E:\PCL.exe", r"E:\PCL.exe"),
            ] {
                assert_eq!(
                    compatible_launch_path(Path::new(source)),
                    PathBuf::from(expected)
                );
            }
        }
        #[test]
        fn missing_central_program_still_starts_original_with_its_working_directory() {
            use crate::binding::{apply_at, read_binding};
            let directory = std::env::temp_dir().join(format!(
                "packporter-shim-fallback-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&directory).unwrap();
            let launcher = directory.join("launcher.exe");
            let shim = directory.join("packporter-shim.exe");
            let app = directory.join("packporter.exe");
            let windows = std::env::var_os("SystemRoot").unwrap();
            fs::copy(PathBuf::from(windows).join("System32/cmd.exe"), &launcher).unwrap();
            fs::write(&shim, b"fixture shim").unwrap();
            fs::write(&app, b"fixture app").unwrap();
            apply_at(
                true,
                std::slice::from_ref(&launcher),
                &app,
                &shim,
                &directory,
            )
            .unwrap();
            fs::remove_file(&app).unwrap();
            fs::remove_file(&shim).unwrap();
            let binding = read_binding(&launcher).unwrap().unwrap();
            let result = handoff(
                binding,
                vec![
                    "/d".into(),
                    "/c".into(),
                    "echo fallback>launched.txt".into(),
                ],
            );
            assert!(result.unwrap_err().contains("原启动器已启动"));
            let output = directory.join("launched.txt");
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !output.is_file() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(fs::read_to_string(output).unwrap().trim(), "fallback");
            // cmd 已写完输出后可能尚未释放映像句柄，清理仅等待本测试的临时进程退出。
            while let Err(error) = fs::remove_dir_all(&directory) {
                assert!(std::time::Instant::now() < deadline, "{error}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        #[test]
        fn quoting_preserves_empty_unicode_quotes_and_trailing_slash() {
            assert_eq!(String::from_utf16(&quote(OsStr::new(""))).unwrap(), "\"\"");
            assert_eq!(
                String::from_utf16(&quote(OsStr::new("路径 空格\\"))).unwrap(),
                "\"路径 空格\\\\\""
            );
            assert_eq!(
                String::from_utf16(&quote(OsStr::new("a\\\"b"))).unwrap(),
                "\"a\\\\\\\"b\""
            );
        }
        #[test]
        fn command_rejects_nul() {
            assert!(command_line(OsStr::new("test.exe"), &[OsString::from("a\0b")]).is_err());
        }
        #[test]
        fn empty_job_has_no_launchers() {
            let name = wide(OsStr::new(&format!(
                "Local\\PackPorter.Test.Shim.{}",
                std::process::id()
            )));
            let job = Handle(unsafe { CreateJobObjectW(null(), name.as_ptr()) });
            assert!(!job.0.is_null());
            assert!(members(&job).unwrap().is_empty());
        }
    }
}
