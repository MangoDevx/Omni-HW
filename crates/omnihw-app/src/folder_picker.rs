//! "Select folder" dialog, run on its own thread so the UI keeps painting while it's open.

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

/// The desktop's folder chooser (through the XDG desktop portal).
fn pick(start: &std::path::Path) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Choose where to save recordings and reports")
        .set_directory(start)
        .pick_folder()
}
