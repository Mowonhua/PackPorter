//! 文件职责：托管系统托盘资源，并向应用装配层提供显示和退出请求。
//! 定义范围：托盘生命周期、原生事件适配与已有实例唤起；不决定应用退出策略。

/// 结构职责：承载用户从托盘发起的应用级请求。
/// 字段说明：Show 请求显示主窗口；Quit 请求显式退出应用。
/// 约束条件：动作由创建托盘的线程按接收顺序消费。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    Quit,
}

/// 结构职责：持有托盘的原生资源及尚未消费的请求。
/// 字段说明：原生窗口与事件状态只属于创建线程。
/// 约束条件：必须在创建线程轮询和销毁；创建不要求主窗口可见。
pub struct Tray {
    #[cfg(windows)]
    native: native::NativeTray,
}

impl Tray {
    /// 函数职责：创建隐藏消息窗口并注册托盘图标。
    /// 输入说明：应由应用主线程调用一次。
    /// 输出说明：失败返回原因，调用方必须保留可见窗口作为退路。
    /// 实现思路：由平台适配器申请资源，并在销毁时成对释放。
    pub fn new() -> Result<Self, String> {
        #[cfg(windows)]
        {
            native::NativeTray::new().map(|native| Self { native })
        }
        #[cfg(not(windows))]
        {
            Err("当前平台不支持系统托盘".into())
        }
    }

    /// 函数职责：取出一个待处理的托盘动作。
    /// 输入说明：由创建线程的事件循环定期调用。
    /// 输出说明：没有待处理动作时返回 None，不阻塞。
    /// 实现思路：从原生消息过程写入的队列读取。
    pub fn poll(&self) -> Option<TrayAction> {
        #[cfg(windows)]
        {
            self.native.poll()
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}

/// 函数职责：请求当前桌面上的已有 PackPorter 实例显示窗口。
/// 输入说明：仅在单实例检查发现已有实例且用户手动启动时调用。
/// 输出说明：返回是否成功投递请求；不等待已有实例响应。
/// 实现思路：查找专用隐藏窗口并投递独立消息。
pub fn request_show_existing() -> bool {
    #[cfg(windows)]
    {
        native::request_show_existing()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
mod native {
    use super::TrayAction;
    use std::{
        cell::RefCell, collections::VecDeque, marker::PhantomData, ptr::null, rc::Rc,
        sync::OnceLock,
    };
    use windows_sys::{
        w,
        Win32::{
            Foundation::*,
            System::LibraryLoader::GetModuleHandleW,
            UI::{Shell::*, WindowsAndMessaging::*},
        },
    };

    const CLASS_NAME: windows_sys::core::PCWSTR = w!("PackPorter.Tray.Window.v1");
    const TRAY_MESSAGE: u32 = WM_APP + 1;
    const SHOW_MESSAGE: u32 = WM_APP + 2;
    const SHOW_COMMAND: usize = 1;
    const QUIT_COMMAND: usize = 2;

    /// 状态地址在原生窗口存活期间固定；消息过程只短暂借用队列，避免菜单模态循环重入。
    struct State {
        actions: RefCell<VecDeque<TrayAction>>,
        icon: HICON,
        taskbar_created: u32,
    }

    pub(super) struct NativeTray {
        pub(super) hwnd: HWND,
        state: Box<State>,
        // Win32 窗口必须由创建线程销毁，Rc 标记禁止将守卫移交其他线程。
        _thread_bound: PhantomData<Rc<()>>,
    }

    impl NativeTray {
        pub(super) fn new() -> Result<Self, String> {
            static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
            REGISTERED
                .get_or_init(|| unsafe {
                    let class = WNDCLASSW {
                        lpfnWndProc: Some(window_proc),
                        hInstance: GetModuleHandleW(null()),
                        lpszClassName: CLASS_NAME,
                        ..std::mem::zeroed()
                    };
                    if RegisterClassW(&class) == 0 {
                        return Err(format!(
                            "无法注册托盘窗口类：{}",
                            std::io::Error::last_os_error()
                        ));
                    }
                    Ok(())
                })
                .clone()?;
            unsafe {
                let instance = GetModuleHandleW(null());
                // winresource 默认将应用图标写入资源 1；LoadIconW 返回共享图标，不能 DestroyIcon。
                let mut icon = LoadIconW(instance, 1usize as _);
                if icon.is_null() {
                    icon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
                }
                if icon.is_null() {
                    return Err("无法加载托盘图标".into());
                }
                let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
                if taskbar_created == 0 {
                    return Err("无法注册托盘恢复消息".into());
                }
                let mut state = Box::new(State {
                    actions: RefCell::new(VecDeque::new()),
                    icon,
                    taskbar_created,
                });
                // 使用不可见顶层窗口而非 HWND_MESSAGE，才能接收 Explorer 重启后的广播。
                let hwnd = CreateWindowExW(
                    0,
                    CLASS_NAME,
                    w!("PackPorter"),
                    WS_OVERLAPPED,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    instance,
                    null(),
                );
                if hwnd.is_null() {
                    return Err(format!(
                        "无法创建托盘窗口：{}",
                        std::io::Error::last_os_error()
                    ));
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&mut *state as *mut State) as isize);
                let tray = Self {
                    hwnd,
                    state,
                    _thread_bound: PhantomData,
                };
                if !add_icon(hwnd, &tray.state) {
                    return Err("无法向 Windows 通知区域添加托盘图标".into());
                }
                Ok(tray)
            }
        }

        pub(super) fn poll(&self) -> Option<TrayAction> {
            // 通常由 Slint 的主循环分发；显式泵送本窗口消息也支持主窗口从未显示的启动路径。
            unsafe {
                let mut message: MSG = std::mem::zeroed();
                while PeekMessageW(&mut message, self.hwnd, 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            self.state.actions.borrow_mut().pop_front()
        }
    }

    impl Drop for NativeTray {
        fn drop(&mut self) {
            unsafe {
                let data = icon_data(self.hwnd, &self.state);
                Shell_NotifyIconW(NIM_DELETE, &data);
                // 清除原生借用后销毁窗口，随后 Box 才可释放；不向 Slint 投递 WM_QUIT。
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                DestroyWindow(self.hwnd);
            }
        }
    }

    fn icon_data(hwnd: HWND, state: &State) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = TRAY_MESSAGE;
        data.hIcon = state.icon;
        for (target, value) in data.szTip.iter_mut().zip("PackPorter".encode_utf16()) {
            *target = value;
        }
        data
    }

    unsafe fn add_icon(hwnd: HWND, state: &State) -> bool {
        // 保留默认通知协议，lParam 直接携带鼠标消息编号。
        Shell_NotifyIconW(NIM_ADD, &icon_data(hwnd, state)) != 0
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let state = &*pointer;
        if message == state.taskbar_created {
            add_icon(hwnd, state);
            return 0;
        }
        let action = match message {
            SHOW_MESSAGE => Some(TrayAction::Show),
            WM_COMMAND => match wparam & 0xffff {
                SHOW_COMMAND => Some(TrayAction::Show),
                QUIT_COMMAND => Some(TrayAction::Quit),
                _ => None,
            },
            TRAY_MESSAGE => match lparam as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => Some(TrayAction::Show),
                WM_RBUTTONUP => show_menu(hwnd),
                _ => None,
            },
            _ => return DefWindowProcW(hwnd, message, wparam, lparam),
        };
        if let Some(action) = action {
            state.actions.borrow_mut().push_back(action);
        }
        0
    }

    unsafe fn show_menu(hwnd: HWND) -> Option<TrayAction> {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return None;
        }
        AppendMenuW(menu, MF_STRING, SHOW_COMMAND, w!("显示 PackPorter"));
        AppendMenuW(menu, MF_STRING, QUIT_COMMAND, w!("退出"));
        let mut position: POINT = std::mem::zeroed();
        GetCursorPos(&mut position);
        // 前台窗口和 WM_NULL 使点击菜单外部能够正确关闭通知区菜单。
        SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            position.x,
            position.y,
            0,
            hwnd,
            null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        match command as usize {
            SHOW_COMMAND => Some(TrayAction::Show),
            QUIT_COMMAND => Some(TrayAction::Quit),
            _ => None,
        }
    }

    pub(super) fn request_show_existing() -> bool {
        unsafe {
            let hwnd = FindWindowW(CLASS_NAME, null());
            !hwnd.is_null() && PostMessageW(hwnd, SHOW_MESSAGE, 0, 0) != 0
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    #[test]
    fn native_tray_dispatches_actions_and_destroys_its_window() {
        let tray = Tray::new().expect("Windows 桌面应允许创建托盘");
        let hwnd = tray.native.hwnd;
        assert!(!hwnd.is_null());
        // 只向本测试创建的窗口发消息，避免机器上正在运行的应用接收测试退出请求。
        unsafe {
            PostMessageW(hwnd, WM_APP + 2, 0, 0);
        }
        assert_eq!(tray.poll(), Some(TrayAction::Show));
        unsafe {
            PostMessageW(hwnd, WM_APP + 1, 1, WM_LBUTTONUP as isize);
        }
        assert_eq!(tray.poll(), Some(TrayAction::Show));
        unsafe {
            PostMessageW(hwnd, WM_COMMAND, 2, 0);
        }
        assert_eq!(tray.poll(), Some(TrayAction::Quit));
        assert_eq!(tray.poll(), None);
        drop(tray);
        assert_eq!(unsafe { IsWindow(hwnd) }, 0);
        let recreated = Tray::new().expect("释放后可再次创建托盘");
        drop(recreated);
    }
}
