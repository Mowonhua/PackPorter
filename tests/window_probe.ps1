# 只识别 PackPorter 的可见主窗口，排除 winit 的内部事件窗口和隐藏托盘窗口。
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class TrayRuntimeNative {
    private delegate bool EnumProc(IntPtr hwnd, IntPtr param);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr param);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, UIntPtr wp, IntPtr lp);
    public static IntPtr TrayWindow() { return FindWindow("PackPorter.Tray.Window.v1", null); }
    public static IntPtr MainWindow(int pid) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((h, _) => {
            uint owner; GetWindowThreadProcessId(h, out owner);
            var title = new StringBuilder(512);
            GetWindowText(h, title, 512);
            if (owner == pid && IsWindowVisible(h) && title.ToString() == "PackPorter") { result = h; return false; }
            return true;
        }, IntPtr.Zero);
        return result;
    }
}
"@
