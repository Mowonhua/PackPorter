//! 文件职责：应用实例锁与旧登录启动项清理。
//! 定义范围：窗口互斥凭据、shim 会话计数适配，不创建常驻监视器。

/// 实例存活凭据；调用方必须持有到对应进程入口结束。
pub struct InstanceGuard {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

/// 清理旧版本的当前用户登录启动项；启动 UI 时可重复调用。
pub fn cleanup_legacy_startup() -> Result<(), String> {
    #[cfg(windows)]
    {
        platform::cleanup_legacy_startup()
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

/// None 表示已有窗口；手动入口和 shim 入口使用相同互斥名称。
pub fn acquire_ui_instance() -> Result<Option<InstanceGuard>, String> {
    #[cfg(windows)]
    {
        platform::acquire_instance("Local\\PackPorter.UI")
    }
    #[cfg(not(windows))]
    {
        Ok(Some(InstanceGuard {}))
    }
}

/// 给上一批窗口的安全退出留出短暂时间；超时后由既有窗口观察新增会话。
pub fn acquire_followed_ui_instance() -> Result<Option<InstanceGuard>, String> {
    #[cfg(windows)]
    {
        platform::acquire_instance_with_retry("Local\\PackPorter.UI")
    }
    #[cfg(not(windows))]
    {
        acquire_ui_instance()
    }
}

/// 只计数 shim 会话中的启动器；失败不能被调用方当成零。
pub fn launcher_count() -> Result<usize, String> {
    super::launcher_shim::launcher_count()
}

#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // 互斥对象没有所有权；最后一个引用关闭时释放名称。
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::InstanceGuard;
    use std::{
        ffi::OsStr,
        os::windows::ffi::OsStrExt,
        ptr::{null, null_mut},
    };
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS,
        },
        System::{
            Registry::{
                RegCloseKey, RegDeleteValueW, RegOpenKeyExW, HKEY_CURRENT_USER, KEY_SET_VALUE,
            },
            Threading::CreateMutexW,
        },
    };
    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    pub(super) fn acquire_instance(name: &str) -> Result<Option<InstanceGuard>, String> {
        let name = wide(OsStr::new(name));
        // 名称缓冲区在调用期间有效；不申请互斥所有权，仅以内核对象存活判定单实例。
        let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        let error = unsafe { GetLastError() };
        if handle.is_null() {
            return Err(format!(
                "无法创建实例锁：{}",
                std::io::Error::from_raw_os_error(error as i32)
            ));
        }
        if error == ERROR_ALREADY_EXISTS {
            unsafe {
                CloseHandle(handle);
            }
            return Ok(None);
        }
        Ok(Some(InstanceGuard { handle }))
    }

    pub(super) fn acquire_instance_with_retry(name: &str) -> Result<Option<InstanceGuard>, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let instance = acquire_instance(name)?;
            if instance.is_some() || std::time::Instant::now() >= deadline {
                return Ok(instance);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    pub(super) fn cleanup_legacy_startup() -> Result<(), String> {
        let path = wide(OsStr::new(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        ));
        let name = wide(OsStr::new("PackPorterLauncherMonitor"));
        let mut key = null_mut();
        // 只打开既有键并删除自己的命名值，不创建 Run 键或修改其他软件。
        let result =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_SET_VALUE, &mut key) };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        if result != ERROR_SUCCESS {
            return Err(format!("无法打开旧登录启动设置：{result}"));
        }
        let result = unsafe { RegDeleteValueW(key, name.as_ptr()) };
        unsafe {
            RegCloseKey(key);
        }
        if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        Err(format!("无法清理旧登录启动设置：{result}"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn instance_lock_is_released_when_guard_drops() {
            let name = format!("Local\\PackPorter.Test.{}", std::process::id());
            let guard = acquire_instance(&name).unwrap().unwrap();
            assert!(acquire_instance(&name).unwrap().is_none());
            drop(guard);
            assert!(acquire_instance(&name).unwrap().is_some());
        }
    }
}
