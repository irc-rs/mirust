use windows::Win32::{Foundation::HWND, System::Threading::GetCurrentThreadId, UI::WindowsAndMessaging::GetWindowThreadProcessId};

pub fn is_main_thread(hwnd: HWND) -> bool {
    unsafe {
        let current_thread_id = GetCurrentThreadId();
        let main_thread_id = GetWindowThreadProcessId(hwnd, None);
        current_thread_id == main_thread_id
    }
}
