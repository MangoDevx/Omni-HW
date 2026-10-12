//! Desktop notifications for fired alerts: a balloon from the app's single tray icon (shown as a
//! toast on Windows 10/11, see [`crate::tray`]) plus a taskbar flash when the window isn't focused.

#[cfg(windows)]
mod imp {
    #[repr(C)]
    struct FlashInfo {
        cb_size: u32,
        hwnd: isize,
        flags: u32,
        count: u32,
        timeout: u32,
    }

    const FLASHW_ALL_UNTIL_FOREGROUND: u32 = 0x3 | 0xC;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn FlashWindowEx(info: *mut FlashInfo) -> i32;
        fn GetForegroundWindow() -> isize;
    }

    pub struct Notifier {
        hwnd: isize,
    }

    impl Notifier {
        pub fn new(hwnd: isize) -> Self {
            Notifier { hwnd }
        }

        pub fn show(&mut self, title: &str, message: &str) {
            crate::tray::balloon(title, message);
            // SAFETY: trivially safe.
            if self.hwnd != 0 && unsafe { GetForegroundWindow() } != self.hwnd {
                let mut f = FlashInfo {
                    cb_size: size_of::<FlashInfo>() as u32,
                    hwnd: self.hwnd,
                    flags: FLASHW_ALL_UNTIL_FOREGROUND,
                    count: 3,
                    timeout: 0,
                };
                // SAFETY: valid struct.
                unsafe { FlashWindowEx(&mut f) };
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
