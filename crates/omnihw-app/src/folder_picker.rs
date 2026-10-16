//! Windows "Select folder" dialog, run on its own thread so the UI keeps painting while it's open.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// A folder dialog that may be open; poll [`FolderPicker::take`] each frame for the result.
#[derive(Default, Clone)]
pub struct FolderPicker {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    open: bool,
    picked: Option<PathBuf>,
}

impl FolderPicker {
    pub fn is_open(&self) -> bool {
        self.state.lock().map(|s| s.open).unwrap_or(false)
    }

    /// Open the dialog starting in `start`, unless one is already open.
    pub fn open(&self, ctx: &egui::Context, start: PathBuf) {
        {
            let Ok(mut s) = self.state.lock() else { return };
            if s.open {
                return;
            }
            s.open = true;
        }
        let state = self.state.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let picked = pick(&start);
            if let Ok(mut s) = state.lock() {
                s.open = false;
                s.picked = picked;
            }
            ctx.request_repaint();
        });
    }

    /// The folder chosen since the last call, if any (None also when the dialog was cancelled).
    pub fn take(&self) -> Option<PathBuf> {
        self.state.lock().ok()?.picked.take()
    }
}

#[cfg(windows)]
fn pick(start: &std::path::Path) -> Option<PathBuf> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
        CoInitializeEx, CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
        SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
    };
    use windows::core::{HSTRING, PWSTR, w};

    /// COM initialised for this (new, short-lived) thread; uninitialised on drop.
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            // SAFETY: pairs the successful CoInitializeEx below, on the same thread.
            unsafe { CoUninitialize() };
        }
    }
    /// A string allocated by COM; freed on drop.
    struct CoString(PWSTR);
    impl Drop for CoString {
        fn drop(&mut self) {
            // SAFETY: allocated by the shell with CoTaskMemAlloc, freed exactly once.
            unsafe { CoTaskMemFree(Some(self.0.0 as *const _)) };
        }
    }

    // SAFETY: plain COM initialisation for this thread; undone by `Com`'s drop.
    if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_err() {
        return None;
    }
    let _com = Com;
    // SAFETY: standard IFileOpenDialog use on the COM thread set up above. Every interface is
    // dropped at the end of this block, before `_com` uninitialises COM.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let opts = dialog.GetOptions().ok()?;
        dialog
            .SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)
            .ok()?;
        let _ = dialog.SetTitle(w!("Choose where to save recordings and reports"));
        if let Ok(item) =
            SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(start.as_os_str()), None)
        {
            let _ = dialog.SetFolder(&item);
        }
        // Errors when cancelled.
        dialog.Show(None).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = CoString(item.GetDisplayName(SIGDN_FILESYSPATH).ok()?);
        name.0.to_string().ok().map(PathBuf::from)
    }
}

#[cfg(not(windows))]
fn pick(_start: &std::path::Path) -> Option<PathBuf> {
    None
}
