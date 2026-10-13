//! Desktop notifications for fired alerts: a balloon from the app's single tray icon (shown as a
//! toast on Windows 10/11, see [`crate::tray`]) plus a taskbar flash when the window isn't focused.

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        FLASHW_ALL, FLASHW_TIMERNOFG, FLASHWINFO, FlashWindowEx, GetForegroundWindow,
    };

    pub struct Notifier {
        hwnd: isize,
    }

    impl Notifier {
        pub fn new(hwnd: isize) -> Self {
            Notifier { hwnd }
        }

        pub fn show(&mut self, title: &str, message: &str) {
            crate::tray::balloon(title, message);
            let hwnd = HWND(self.hwnd as *mut _);
            // SAFETY: trivially safe.
            if self.hwnd != 0 && unsafe { GetForegroundWindow() } != hwnd {
                let f = FLASHWINFO {
                    cbSize: size_of::<FLASHWINFO>() as u32,
                    hwnd,
                    dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
                    uCount: 3,
                    dwTimeout: 0,
                };
                // SAFETY: valid struct.
                let _ = unsafe { FlashWindowEx(&f) };
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct Notifier;
    impl Notifier {
        pub fn new(_hwnd: isize) -> Self {
            Notifier
        }
        pub fn show(&mut self, _title: &str, _message: &str) {}
    }
}

pub use imp::Notifier;
