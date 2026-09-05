//! 文件职责：Windows 无边框窗口的原生镶边：命中测试子类化提供标题栏拖动、
//! 八向边缘缩放与双击最大化/还原，并开启 DWM 圆角与投影，让自绘窗口保持系统级手感。
//! 平台约束：仅 Windows 编译；窗口几何来自装配层注入的探针闭包，本模块不依赖 UI 类型。

use std::cell::Cell;
use std::time::Instant;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND,
};
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsZoomed, PostMessageW, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION,
    HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, SC_MAXIMIZE, SC_RESTORE,
    WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCDESTROY, WM_SYSCOMMAND,
};

/// 标题栏命中几何（逻辑像素，由 UI 线程探针在命中测试时提供）。
#[derive(Clone, Copy, Default, Debug)]
pub struct TitlebarGeometry {
    /// 标题栏整条高度：条带内按下方区域细分。
    pub caption_height: f32,
    /// 窗口控制区（最小化/最大化/关闭）起点 x：自此到窗口右缘恒为客户区。
    pub controls_x: f32,
    /// 条带内嵌客户区片段（紧随标题的设置按钮）[start, end)：其间放行点击、不可拖动。
    pub inline_client: (f32, f32),
}

/// 标题栏几何探针：命中测试时在窗口 UI 线程被调用。
/// 窗口销毁后建议返回 Default（全零 = 无可拖动区，命中一律客户区）。
pub type GeometryProbe = Box<dyn Fn() -> TitlebarGeometry>;

/// 原生窗口句柄类型（对装配层屏蔽 windows-sys 依赖细节）。
pub use windows_sys::Win32::Foundation::HWND as NativeWindow;

/// 逻辑像素下的窗口边缘可抓取宽度（系统默认约 8 物理像素 @100% 缩放）。
const RESIZE_BORDER_LOGICAL: f32 = 6.0;
/// 标题栏双击判定的位移容差（物理像素）。
const DOUBLE_CLICK_SLOP: i32 = 4;
/// 子类实例 ID：单窗口应用，取值仅要求进程内唯一。
const SUBCLASS_ID: usize = 0x5043_4357;

/**
 * 结构职责：子类过程的随窗口数据：几何探针 + 标题栏双击检测状态。
 * 字段说明：仅在窗口所属 UI 线程访问；按 press 状态用 Cell 防止模态移动循环重入导致的多重可变借用。
 */
struct ChromeState {
    probe: GeometryProbe,
    // 上一次标题栏按下 (时刻, 屏幕 x, 屏幕 y)；winit 窗口类未注册 CS_DBLCLKS，系统不派发双击消息。
    last_caption_press: Cell<Option<(Instant, i32, i32)>>,
}

/**
 * 函数职责：为无边框窗口安装原生镶边（命中测试子类 + DWM 圆角/投影）。
 * 输入说明：hwnd 为原生窗口句柄；probe 为标题栏几何探针。
 * 输出说明：无返回值；DWM 属性设置失败静默忽略（如 Win10 无圆角属性）。
 *
 * # Safety
 * hwnd 必须是当前 UI 线程拥有的有效窗口句柄；probe 仅在窗口 UI 线程被调用。
 * 约束条件：必须在窗口所属 UI 线程调用，且每个窗口仅安装一次。
 */
pub unsafe fn install(hwnd: HWND, probe: GeometryProbe) {
    let state = Box::new(ChromeState { probe, last_caption_press: Cell::new(None) });
    unsafe {
        // Win11 圆角；Win10 无此属性会返回错误码，属预期降级。
        let corner_preference: i32 = DWMWCP_ROUND;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &corner_preference as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
        // 底边延伸 1px DWM 边框唤回系统投影；不改客户区矩形，命中测试不受影响。
        let margins = MARGINS {
            cxLeftWidth: 0,
            cxRightWidth: 0,
            cyTopHeight: 0,
            cyBottomHeight: 1,
        };
        DwmExtendFrameIntoClientArea(hwnd, &margins);
        SetWindowSubclass(hwnd, Some(chrome_proc), SUBCLASS_ID, Box::into_raw(state) as usize);
    }
}

/**
 * 函数职责：子类窗口过程：拦截命中测试与标题栏按下，其余消息放行原链（winit）。
 * 实现思路：命中测试按「八向边缘 > 标题栏 > 客户区」顺序判定；
 *           标题栏按下先做双击检测（命中则投递系统最大化/还原命令），否则交默认过程
 *           进入系统移动循环，从而保留贴边分屏与最大化后拖动还原的原生行为。
 */
unsafe extern "system" fn chrome_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    uid: usize,
    data: usize,
) -> LRESULT {
    match msg {
        WM_NCDESTROY => {
            RemoveWindowSubclass(hwnd, Some(chrome_proc), uid);
            drop(Box::from_raw(data as *mut ChromeState));
        }
        WM_NCHITTEST => {
            let state = &*(data as *const ChromeState);
            if let Some(code) = hit_test(hwnd, &state.probe, lp) {
                return code as LRESULT;
            }
        }
        WM_NCLBUTTONDOWN if wp as u32 == HTCAPTION => {
            let state = &*(data as *const ChromeState);
            if handle_caption_press(hwnd, state, lp) {
                return 0;
            }
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

/**
 * 函数职责：处理标题栏按下：双击检测，非双击放行给默认过程启动系统移动循环。
 * 输入说明：hwnd 为窗口句柄；state 为子类数据；lp 为消息携带的屏幕坐标。
 * 输出说明：true 表示双击已消费（已投递最大化/还原命令），调用方直接返回；
 *           false 表示按单击处理，需继续走 DefSubclassProc。
 * 实现思路：时间间隔与位移均落在系统双击参数内判定为双击（窗口类无 CS_DBLCLKS）。
 */
unsafe fn handle_caption_press(hwnd: HWND, state: &ChromeState, lp: LPARAM) -> bool {
    let now = Instant::now();
    let (sx, sy) = cursor_screen_pos(lp);
    if let Some((last, last_x, last_y)) = state.last_caption_press.get() {
        if now.duration_since(last).as_millis() as u32 <= GetDoubleClickTime()
            && (sx - last_x).abs() <= DOUBLE_CLICK_SLOP
            && (sy - last_y).abs() <= DOUBLE_CLICK_SLOP
        {
            state.last_caption_press.set(None);
            let command = if IsZoomed(hwnd) != 0 { SC_RESTORE } else { SC_MAXIMIZE };
            PostMessageW(hwnd, WM_SYSCOMMAND, command as usize, 0);
            return true;
        }
    }
    state.last_caption_press.set(Some((now, sx, sy)));
    false
}

/**
 * 函数职责：命中测试：把光标位置映射为系统命中码（边缘缩放/标题栏拖动/客户区）。
 * 输入说明：hwnd 为窗口句柄；probe 提供标题栏几何；lp 为消息携带的屏幕坐标。
 * 输出说明：Some(命中码) 由子类过程直接返回；None 表示无法判定，走默认链。
 * 实现思路：无边框窗口客户区与窗口矩形重合，直接以窗口矩形换算窗口内物理坐标；
 *           非最大化先判八向边缘（角优先），再判标题栏条带（控制区起点之前）。
 */
unsafe fn hit_test(hwnd: HWND, probe: &GeometryProbe, lp: LPARAM) -> Option<u32> {
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    if GetWindowRect(hwnd, &mut rect) == 0 {
        return None;
    }
    let (sx, sy) = cursor_screen_pos(lp);
    let (x, y) = (sx - rect.left, sy - rect.top);
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    if x < 0 || y < 0 || x >= width || y >= height {
        return Some(HTCLIENT);
    }
    let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
    if IsZoomed(hwnd) == 0 {
        let border = (RESIZE_BORDER_LOGICAL * scale) as i32;
        let (north, south) = (y < border, y >= height - border);
        let (west, east) = (x < border, x >= width - border);
        let code = match (north, south, west, east) {
            (true, _, true, _) => HTTOPLEFT,
            (true, _, _, true) => HTTOPRIGHT,
            (_, true, true, _) => HTBOTTOMLEFT,
            (_, true, _, true) => HTBOTTOMRIGHT,
            (true, _, _, _) => HTTOP,
            (_, true, _, _) => HTBOTTOM,
            (_, _, true, _) => HTLEFT,
            (_, _, _, true) => HTRIGHT,
            _ => 0,
        };
        if code != 0 {
            return Some(code);
        }
    }
    let geo = probe();
    let caption_px = (geo.caption_height * scale) as i32;
    if y < caption_px {
        // 控制区之前、内嵌按钮片段之外即标题栏拖动区。
        let (inline_start, inline_end) = (
            (geo.inline_client.0 * scale) as i32,
            (geo.inline_client.1 * scale) as i32,
        );
        let controls_px = (geo.controls_x * scale) as i32;
        if x < controls_px && !(x >= inline_start && x < inline_end) {
            return Some(HTCAPTION);
        }
    }
    Some(HTCLIENT)
}

/**
 * 函数职责：从消息 lParam 解出光标屏幕坐标（带符号 16 位组装，兼容多显示器负坐标）。
 */
fn cursor_screen_pos(lp: LPARAM) -> (i32, i32) {
    let bits = lp as usize;
    let x = (bits & 0xFFFF) as u16 as i16 as i32;
    let y = ((bits >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}
