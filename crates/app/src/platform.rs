//! The one Windows call the window itself needs; everything else the app
//! asks of Windows lives in `vmm-engine`'s `host` module.

#[cfg(windows)]
pub fn is_minimized(hwnd: windows::Win32::Foundation::HWND) -> bool {
    // SAFETY: plain window-state query.
    unsafe { windows::Win32::UI::WindowsAndMessaging::IsIconic(hwnd).as_bool() }
}
