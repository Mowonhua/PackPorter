//! 文件职责：独立启动器包装程序入口。
//! 定义范围：命令行解析、错误呈现与 shim 基础设施调用。
#![cfg_attr(windows, windows_subsystem = "windows")]
use packporter_launcher::shim::{self as launcher_shim, ShimLaunch};
use std::{ffi::OsString, path::PathBuf};

const USAGE: &str = "用法：packporter-shim --launcher <启动器.exe 或 HMCL.jar> [--java <javaw.exe>] [--app <packporter.exe>] [-- <启动器参数...>]";

/// 分隔符后的参数原样转发，避免启动器自己的选项被 shim 消费。
fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<ShimLaunch, String> {
    let mut arguments = arguments.into_iter();
    let mut launcher = None;
    let mut java = None;
    let mut app = None;
    let mut forwarded = Vec::new();
    while let Some(argument) = arguments.next() {
        if argument == "--" {
            forwarded.extend(arguments);
            break;
        }
        let destination = if argument == "--launcher" {
            &mut launcher
        } else if argument == "--java" {
            &mut java
        } else if argument == "--app" {
            &mut app
        } else {
            return Err(format!("未知参数：{}\n{USAGE}", argument.to_string_lossy()));
        };
        if destination.is_some() {
            return Err(format!("参数重复：{}", argument.to_string_lossy()));
        }
        let value = arguments
            .next()
            .filter(|value| !value.is_empty() && !value.to_string_lossy().starts_with("--"))
            .ok_or_else(|| format!("参数 {} 缺少路径\n{USAGE}", argument.to_string_lossy()))?;
        *destination = Some(PathBuf::from(value));
    }
    Ok(ShimLaunch {
        launcher: launcher.ok_or(USAGE)?,
        java,
        app,
        arguments: forwarded,
    })
}

fn show_error(message: &str) {
    eprintln!("{message}");
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "PackPorter 启动器入口"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

fn main() {
    let result = std::env::current_exe()
        .map_err(|error| error.to_string())
        .and_then(|executable| {
            // 受管原入口的所有参数属于启动器，必须先识别绑定再解析 shim 自身选项。
            match packporter_launcher::binding::read_binding(&executable)? {
                Some(binding) => {
                    launcher_shim::handoff(binding, std::env::args_os().skip(1).collect())
                }
                None => parse(std::env::args_os().skip(1)).and_then(launcher_shim::run),
            }
        });
    if let Err(error) = result {
        show_error(&error);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }
    #[test]
    fn preserves_forwarded_arguments_including_shim_option_names() {
        let launch = parse(args(&[
            "--launcher",
            "C:\\启动器 目录\\PCL2.exe",
            "--",
            "--java",
            "",
            "a\"b",
        ]))
        .unwrap();
        assert_eq!(launch.launcher, PathBuf::from("C:\\启动器 目录\\PCL2.exe"));
        assert_eq!(launch.arguments, args(&["--java", "", "a\"b"]));
    }
    #[test]
    fn explicit_app_location_is_independent_of_launcher_directory() {
        let launch = parse(args(&[
            "--launcher",
            "E:\\启动器\\PCL2.bak.exe",
            "--app",
            "D:\\软件\\packporter.exe",
            "--",
            "--app",
            "原始参数",
        ]))
        .unwrap();
        assert_eq!(launch.app, Some(PathBuf::from("D:\\软件\\packporter.exe")));
        assert_eq!(launch.arguments, args(&["--app", "原始参数"]));
    }
    #[test]
    fn rejects_missing_duplicate_and_unknown_options() {
        for values in [
            vec![],
            vec!["--launcher"],
            vec!["--launcher", "--java"],
            vec!["--launcher", "a", "--launcher", "b"],
            vec!["--launcher", "a", "unknown"],
        ] {
            assert!(parse(args(&values)).is_err());
        }
    }
}
