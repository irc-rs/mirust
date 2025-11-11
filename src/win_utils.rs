use windows::Win32::{
    Foundation::{HWND, LPARAM},
    System::Threading::GetCurrentProcessId,
    UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetWindowThreadProcessId},
};
use windows::core::BOOL;

/// Find the mIRC32 window for the current process.
/// This applies only to mIRC v5.6, 5.61, 5.7, and 5.71 only.
pub(crate) fn get_mirc32_hwnd() -> HWND {
    struct SearchContext {
        pid: u32,
        result: HWND,
    }

    unsafe {
        let pid = GetCurrentProcessId();
        let mut context = SearchContext {
            pid,
            result: HWND::default(),
        };

        unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            unsafe {
                let context = &mut *(lparam.0 as *mut SearchContext);
                let mut window_pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
                if window_pid != context.pid {
                    return true.into();
                }

                let mut class_name = [0u16; 256];
                let len = GetClassNameW(hwnd, &mut class_name);
                if len == 0 {
                    return true.into();
                }

                let name = String::from_utf16_lossy(&class_name[..len as usize]);
                if name == "mIRC32" {
                    context.result = hwnd;
                    return false.into();
                }

                true.into()
            }
        }

        let _ = EnumWindows(
            Some(enum_windows_proc),
            LPARAM(&mut context as *mut _ as isize),
        );
        context.result
    }
}
