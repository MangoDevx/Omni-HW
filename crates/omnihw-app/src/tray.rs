//! The single notification-area (system tray) icon for Omni-HW.
//!
//! A background thread owns a hidden window and message loop that receives the icon's mouse
//! messages: click to restore the main window, right-click for Open / Exit. Alert balloons
//! (shown as toasts on Windows 10/11) go through the same icon, so there is never more than one.
//! The icon is shown while "Minimize to the system tray" is on, or once a balloon needs it.

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::sync::{Mutex, OnceLock};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{
        NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NOTIFYICONDATAW, Shell_NotifyIconW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetCursorPos, GetMessageW, IDI_APPLICATION, LoadIconW, MF_STRING, MSG,
        PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SW_HIDE,
        SW_RESTORE, SendMessageW, SetForegroundWindow, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON,
        TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE,
        WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL,
        WM_RBUTTONUP, WNDCLASSEXW,
    };
    use windows::core::{PCWSTR, w};

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

    /// HWNDs cross threads as `isize` (the crate's `HWND` wraps a raw pointer, so it isn't `Send`).
    fn hwnd(raw: isize) -> HWND {
        HWND(raw as *mut _)
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

    fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            ..Default::default()
        }
    }

    /// Add the icon if it isn't there yet (tray thread only).
    fn add_icon(hwnd: HWND) {
        if ICON_ADDED.load(Ordering::Relaxed) {
            return;
        }
        let mut d = icon_data(hwnd);
        d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        d.uCallbackMessage = MSG_TRAY;
        // SAFETY: the executable's first icon resource if it has one, else the stock app icon.
        d.hIcon = unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).ok().map(HINSTANCE::from);
            LoadIconW(instance, PCWSTR(1 as _))
                .or_else(|_| LoadIconW(None, IDI_APPLICATION))
                .unwrap_or_default()
        };
        copy_wide(&mut d.szTip, "Omni-HW");
        // SAFETY: fully initialized struct.
        if unsafe { Shell_NotifyIconW(NIM_ADD, &d) }.as_bool() {
            ICON_ADDED.store(true, Ordering::Relaxed);
        }
    }

    fn remove_icon(hwnd: HWND) {
        if ICON_ADDED.swap(false, Ordering::Relaxed) {
            let d = icon_data(hwnd);
            // SAFETY: removes the icon this window added.
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &d) };
        }
    }

    fn show_pending_balloon(hwnd: HWND) {
        let Some((title, message)) = PENDING_BALLOON
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        else {
            return;
        };
        add_icon(hwnd);
        let mut d = icon_data(hwnd);
        d.uFlags = NIF_INFO;
        d.dwInfoFlags = NIIF_WARNING;
        copy_wide(&mut d.szInfoTitle, &title);
        copy_wide(&mut d.szInfo, &message);
        // SAFETY: valid struct for the icon added above.
        let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
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
            let _ = ShowWindow(hwnd(main), SW_RESTORE);
            let _ = SetForegroundWindow(hwnd(main));
        }
        if let Some(ctx) = CTX.get() {
            ctx.request_repaint();
        }
    }

    fn context_menu(tray: HWND) {
        // SAFETY: standard popup-menu sequence; the menu is destroyed before returning.
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, w!("Open Omni-HW"));
            let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, w!("Exit"));
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            // Without this the menu doesn't close when clicking elsewhere.
            let _ = SetForegroundWindow(tray);
            let cmd = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD,
                pt.x,
                pt.y,
                None,
                tray,
                None,
            );
            let _ = PostMessageW(Some(tray), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            match cmd.0 as usize {
                ID_OPEN => restore_main(),
                ID_EXIT => {
                    let main = MAIN_HWND.load(Ordering::Relaxed);
                    if main != 0 {
                        // Closing the main window ends the app; OmniApp's Drop removes the icon.
                        let _ = PostMessageW(Some(hwnd(main)), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
                _ => {}
            }
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            MSG_TRAY => match lparam.0 as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => restore_main(),
                WM_RBUTTONUP | WM_CONTEXTMENU => context_menu(hwnd),
                _ => {}
            },
            MSG_BALLOON => show_pending_balloon(hwnd),
            MSG_SET_ICON => {
                if wparam.0 != 0 {
                    add_icon(hwnd);
                } else {
                    remove_icon(hwnd);
                }
            }
            MSG_SHUTDOWN => {
                remove_icon(hwnd);
                // SAFETY: our own window, destroyed on its own thread.
                let _ = unsafe { DestroyWindow(hwnd) };
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
        LRESULT(0)
    }

    fn run_thread() {
        let class = w!("OmniHwTrayWindow");
        // SAFETY: class struct and strings outlive the calls; window lives on this thread.
        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null())
                .map(HINSTANCE::from)
                .unwrap_or_default();
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);
            TASKBAR_CREATED.store(
                RegisterWindowMessageW(w!("TaskbarCreated")),
                Ordering::Relaxed,
            );
            // A hidden top-level window (not message-only) so it receives the TaskbarCreated broadcast.
            let Ok(tray) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!("Omni-HW tray"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            ) else {
                return;
            };
            TRAY_HWND.store(tray.0 as isize, Ordering::Relaxed);
            if ICON_WANTED.load(Ordering::Relaxed) {
                add_icon(tray);
            }
            show_pending_balloon(tray);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&msg);
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
            let _ = unsafe {
                PostMessageW(
                    Some(hwnd(tray)),
                    MSG_SET_ICON,
                    WPARAM(usize::from(on)),
                    LPARAM(0),
                )
            };
        }
    }

    /// Show a notification balloon (a toast on Windows 10/11) from the tray icon.
    pub fn balloon(title: &str, message: &str) {
        *PENDING_BALLOON.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((title.to_string(), message.to_string()));
        let tray = TRAY_HWND.load(Ordering::Relaxed);
        if tray != 0 {
            // SAFETY: posts to our own window; handled on the tray thread.
            let _ = unsafe { PostMessageW(Some(hwnd(tray)), MSG_BALLOON, WPARAM(0), LPARAM(0)) };
        }
        // Otherwise the thread shows it as soon as its window exists.
    }

    /// Hide the main window (minimized to the tray).
    pub fn hide_main_window() {
        let main = MAIN_HWND.load(Ordering::Relaxed);
        if main != 0 {
            // SAFETY: plain window call on the app's main window.
            let _ = unsafe { ShowWindow(hwnd(main), SW_HIDE) };
        }
    }

    /// Remove the icon and stop the thread. Blocks until the icon is gone.
    pub fn shutdown() {
        let tray = TRAY_HWND.load(Ordering::Relaxed);
        if tray != 0 {
            // SAFETY: synchronous message to our own window on the tray thread.
            unsafe { SendMessageW(hwnd(tray), MSG_SHUTDOWN, None, None) };
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
