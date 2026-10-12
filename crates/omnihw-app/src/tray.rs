//! The single notification-area (system tray) icon for Omni-HW.
//!
//! A background thread owns a hidden window and message loop that receives the icon's mouse
//! messages: click to restore the main window, right-click for Open / Exit. Alert balloons
//! (shown as toasts on Windows 10/11) go through the same icon, so there is never more than one.
//! The icon is shown while "Minimize to the system tray" is on, or once a balloon needs it.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::sync::{Mutex, OnceLock};

    type Hwnd = isize;

    #[repr(C)]
    struct NotifyIconData {
        cb_size: u32,
        hwnd: Hwnd,
        id: u32,
        flags: u32,
        callback_message: u32,
        icon: *mut c_void,
        tip: [u16; 128],
        state: u32,
        state_mask: u32,
        info: [u16; 256],
        version: u32,
        info_title: [u16; 64],
        info_flags: u32,
        guid: [u8; 16],
        balloon_icon: *mut c_void,
    }

    #[repr(C)]
    struct WndClassEx {
        cb_size: u32,
        style: u32,
        wnd_proc: unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize,
        cls_extra: i32,
        wnd_extra: i32,
        instance: *mut c_void,
        icon: *mut c_void,
        cursor: *mut c_void,
        background: *mut c_void,
        menu_name: *const u16,
        class_name: *const u16,
        icon_small: *mut c_void,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt_x: i32,
        pt_y: i32,
        private: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Point {
        x: i32,
        y: i32,
    }

    const NIM_ADD: u32 = 0;
    const NIM_MODIFY: u32 = 1;
    const NIM_DELETE: u32 = 2;
    const NIF_MESSAGE: u32 = 0x1;
    const NIF_ICON: u32 = 0x2;
    const NIF_TIP: u32 = 0x4;
    const NIF_INFO: u32 = 0x10;
    const NIIF_WARNING: u32 = 0x2;

    const WM_NULL: u32 = 0;
    const WM_DESTROY: u32 = 0x2;
    const WM_CLOSE: u32 = 0x10;
    const WM_COMMAND: u32 = 0x111;
    const WM_LBUTTONUP: u32 = 0x202;
    const WM_LBUTTONDBLCLK: u32 = 0x203;
    const WM_RBUTTONUP: u32 = 0x205;
    const WM_CONTEXTMENU: u32 = 0x7B;
    const WM_APP: u32 = 0x8000;
    /// Mouse messages from the tray icon.
    const MSG_TRAY: u32 = WM_APP + 1;
    /// A balloon is waiting in `PENDING_BALLOON`.
    const MSG_BALLOON: u32 = WM_APP + 2;
    /// Show (wparam 1) or hide (0) the icon.
    const MSG_SET_ICON: u32 = WM_APP + 3;
    /// Remove the icon and end the thread.
    const MSG_SHUTDOWN: u32 = WM_APP + 4;

    const ID_OPEN: usize = 1;
    const ID_EXIT: usize = 2;
    const MF_STRING: u32 = 0;
    const TPM_RIGHTBUTTON: u32 = 0x2;
    const TPM_RETURNCMD: u32 = 0x100;
    const SW_HIDE: i32 = 0;
    const SW_RESTORE: i32 = 9;
    const IDI_APPLICATION: usize = 32512;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn Shell_NotifyIconW(message: u32, data: *mut NotifyIconData) -> i32;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterClassExW(class: *const WndClassEx) -> u16;
        fn CreateWindowExW(
            ex_style: u32,
            class: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: Hwnd,
            menu: *mut c_void,
            instance: *mut c_void,
            param: *mut c_void,
        ) -> Hwnd;
        fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn DestroyWindow(hwnd: Hwnd) -> i32;
        fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
        fn PostMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
        fn SendMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn PostQuitMessage(code: i32);
        fn RegisterWindowMessageW(name: *const u16) -> u32;
        fn LoadIconW(instance: *mut c_void, name: usize) -> *mut c_void;
        fn CreatePopupMenu() -> *mut c_void;
        fn AppendMenuW(menu: *mut c_void, flags: u32, id: usize, text: *const u16) -> i32;
        fn TrackPopupMenu(
            menu: *mut c_void,
            flags: u32,
            x: i32,
            y: i32,
            reserved: i32,
            hwnd: Hwnd,
            rect: *const c_void,
        ) -> i32;
        fn DestroyMenu(menu: *mut c_void) -> i32;
        fn GetCursorPos(point: *mut Point) -> i32;
        fn SetForegroundWindow(hwnd: Hwnd) -> i32;
        fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }

    /// The hidden tray window (0 until the thread has created it).
    static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
    /// The eframe main window.
    static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
    /// Whether the icon should be visible regardless of balloons.
    static ICON_WANTED: AtomicBool = AtomicBool::new(false);
    /// Whether the icon is currently added (only touched on the tray thread).
    static ICON_ADDED: AtomicBool = AtomicBool::new(false);
    static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    static PENDING_BALLOON: Mutex<Option<(String, String)>> = Mutex::new(None);
    static CTX: OnceLock<egui::Context> = OnceLock::new();
    static STARTED: OnceLock<()> = OnceLock::new();

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn copy_wide(dst: &mut [u16], s: &str) {
        let cap = dst.len() - 1;
        let mut n = 0;
        for (d, c) in dst.iter_mut().take(cap).zip(s.encode_utf16()) {
            *d = c;
            n += 1;
        }
        dst[n] = 0;
    }

    fn icon_data(hwnd: Hwnd) -> NotifyIconData {
        // SAFETY: plain-old-data struct; all-zero is a valid initial state.
        let mut d: NotifyIconData = unsafe { std::mem::zeroed() };
        d.cb_size = size_of::<NotifyIconData>() as u32;
        d.hwnd = hwnd;
        d.id = 1;
        d
    }

    /// Add the icon if it isn't there yet (tray thread only).
    fn add_icon(hwnd: Hwnd) {
        if ICON_ADDED.load(Ordering::Relaxed) {
            return;
        }
        let mut d = icon_data(hwnd);
        d.flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        d.callback_message = MSG_TRAY;
        // SAFETY: the executable's first icon resource if it has one, else the stock app icon.
        d.icon = unsafe {
            let own = LoadIconW(GetModuleHandleW(std::ptr::null()), 1);
            if own.is_null() {
                LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
            } else {
                own
            }
        };
        copy_wide(&mut d.tip, "Omni-HW");
        // SAFETY: fully initialized struct.
        if unsafe { Shell_NotifyIconW(NIM_ADD, &mut d) } != 0 {
            ICON_ADDED.store(true, Ordering::Relaxed);
        }
    }

    fn remove_icon(hwnd: Hwnd) {
        if ICON_ADDED.swap(false, Ordering::Relaxed) {
            let mut d = icon_data(hwnd);
            // SAFETY: removes the icon this window added.
            unsafe { Shell_NotifyIconW(NIM_DELETE, &mut d) };
        }
    }

    fn show_pending_balloon(hwnd: Hwnd) {
        let Some((title, message)) = PENDING_BALLOON
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        else {
            return;
        };
        add_icon(hwnd);
        let mut d = icon_data(hwnd);
        d.flags = NIF_INFO;
        d.info_flags = NIIF_WARNING;
        copy_wide(&mut d.info_title, &title);
        copy_wide(&mut d.info, &message);
        // SAFETY: valid struct for the icon added above.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &mut d) };
    }

    /// Bring the main window back from the tray (or from the taskbar) and focus it.
    fn restore_main() {
        let main = MAIN_HWND.load(Ordering::Relaxed);
        if main == 0 {
            return;
        }
        // SAFETY: plain window calls on the app's main window. SW_RESTORE alone both shows a
        // hidden window and un-minimizes it, so no frame can observe a shown-but-minimized state.
        unsafe {
            ShowWindow(main, SW_RESTORE);
            SetForegroundWindow(main);
        }
        if let Some(ctx) = CTX.get() {
            ctx.request_repaint();
        }
    }

    fn context_menu(hwnd: Hwnd) {
        // SAFETY: standard popup-menu sequence; the menu is destroyed before returning.
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            AppendMenuW(menu, MF_STRING, ID_OPEN, wide("Open Omni-HW").as_ptr());
            AppendMenuW(menu, MF_STRING, ID_EXIT, wide("Exit").as_ptr());
            let mut pt = Point::default();
            GetCursorPos(&mut pt);
            // Without this the menu doesn't close when clicking elsewhere.
            SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD,
                pt.x,
                pt.y,
                0,
                hwnd,
                std::ptr::null(),
            );
            PostMessageW(hwnd, WM_NULL, 0, 0);
            DestroyMenu(menu);
            match cmd as usize {
                ID_OPEN => restore_main(),
                ID_EXIT => {
                    let main = MAIN_HWND.load(Ordering::Relaxed);
                    if main != 0 {
                        // Closing the main window ends the app; OmniApp's Drop removes the icon.
                        PostMessageW(main, WM_CLOSE, 0, 0);
                    }
                }
                _ => {}
            }
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: Hwnd,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        match msg {
            MSG_TRAY => match lparam as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => restore_main(),
                WM_RBUTTONUP | WM_CONTEXTMENU => context_menu(hwnd),
                _ => {}
            },
            MSG_BALLOON => show_pending_balloon(hwnd),
            MSG_SET_ICON => {
                if wparam != 0 {
                    add_icon(hwnd);
                } else {
                    remove_icon(hwnd);
                }
            }
            MSG_SHUTDOWN => {
                remove_icon(hwnd);
                // SAFETY: our own window, destroyed on its own thread.
                unsafe { DestroyWindow(hwnd) };
            }
            WM_DESTROY => {
                remove_icon(hwnd);
                TRAY_HWND.store(0, Ordering::Relaxed);
                // SAFETY: trivially safe; ends this thread's message loop.
                unsafe { PostQuitMessage(0) };
            }
            WM_COMMAND => {}
            m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                // Explorer restarted: the old icon is gone.
                ICON_ADDED.store(false, Ordering::Relaxed);
                if ICON_WANTED.load(Ordering::Relaxed) {
                    add_icon(hwnd);
                }
            }
            // SAFETY: default handling for everything else.
            _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
        0
    }

    fn run_thread() {
        let class = wide("OmniHwTrayWindow");
        // SAFETY: class struct and strings outlive the calls; window lives on this thread.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WndClassEx {
                cb_size: size_of::<WndClassEx>() as u32,
                style: 0,
                wnd_proc,
                cls_extra: 0,
                wnd_extra: 0,
                instance,
                icon: std::ptr::null_mut(),
                cursor: std::ptr::null_mut(),
                background: std::ptr::null_mut(),
                menu_name: std::ptr::null(),
                class_name: class.as_ptr(),
                icon_small: std::ptr::null_mut(),
            };
            RegisterClassExW(&wc);
            TASKBAR_CREATED.store(
                RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
                Ordering::Relaxed,
            );
            // A hidden top-level window (not message-only) so it receives the TaskbarCreated broadcast.
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("Omni-HW tray").as_ptr(),
                0,
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                instance,
                std::ptr::null_mut(),
            );
            if hwnd == 0 {
                return;
            }
            TRAY_HWND.store(hwnd, Ordering::Relaxed);
            if ICON_WANTED.load(Ordering::Relaxed) {
                add_icon(hwnd);
            }
            show_pending_balloon(hwnd);
            let mut msg = Msg::default();
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// Start the tray thread (once). The icon appears only if [`set_icon_visible`] or
    /// [`balloon`] asks for it.
    pub fn start(ctx: egui::Context) {
        let _ = CTX.set(ctx);
        STARTED.get_or_init(|| {
            let _ = std::thread::Builder::new()
                .name("omnihw-tray".into())
                .spawn(run_thread);
        });
    }

    pub fn set_main_window(hwnd: isize) {
        MAIN_HWND.store(hwnd, Ordering::Relaxed);
    }

    /// Show or hide the tray icon (it stays while a balloon was shown through it).
    pub fn set_icon_visible(on: bool) {
        if ICON_WANTED.swap(on, Ordering::Relaxed) == on {
            return;
        }
        let tray = TRAY_HWND.load(Ordering::Relaxed);
        if tray != 0 {
            // SAFETY: posts to our own window; handled on the tray thread.
            unsafe { PostMessageW(tray, MSG_SET_ICON, usize::from(on), 0) };
        }
    }

    /// Show a notification balloon (a toast on Windows 10/11) from the tray icon.
    pub fn balloon(title: &str, message: &str) {
        *PENDING_BALLOON.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((title.to_string(), message.to_string()));
        let tray = TRAY_HWND.load(Ordering::Relaxed);
        if tray != 0 {
            // SAFETY: posts to our own window; handled on the tray thread.
            unsafe { PostMessageW(tray, MSG_BALLOON, 0, 0) };
        }
        // Otherwise the thread shows it as soon as its window exists.
    }

    /// Hide the main window (minimized to the tray).
    pub fn hide_main_window() {
        let main = MAIN_HWND.load(Ordering::Relaxed);
        if main != 0 {
            // SAFETY: plain window call on the app's main window.
            unsafe { ShowWindow(main, SW_HIDE) };
        }
    }

    /// Remove the icon and stop the thread. Blocks until the icon is gone.
    pub fn shutdown() {
        let tray = TRAY_HWND.load(Ordering::Relaxed);
        if tray != 0 {
            // SAFETY: synchronous message to our own window on the tray thread.
            unsafe { SendMessageW(tray, MSG_SHUTDOWN, 0, 0) };
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn start(_ctx: egui::Context) {}
    pub fn set_main_window(_hwnd: isize) {}
    pub fn set_icon_visible(_on: bool) {}
    pub fn balloon(_title: &str, _message: &str) {}
    pub fn hide_main_window() {}
    pub fn shutdown() {}
}

pub use imp::{balloon, hide_main_window, set_icon_visible, set_main_window, shutdown, start};
