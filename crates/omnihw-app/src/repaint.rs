//! Waking the UI from other threads.
//!
//! The sampler, the tray icon and the one-shot workers (update check, folder picker, sensor
//! setup) run off the UI thread and can't touch gpui. Each holds a [`Repaint`] and calls
//! [`Repaint::request_repaint`] when it has new data; the app listens on the other end and
//! redraws. Requests made while one is already waiting are merged, so a busy worker can't
//! queue up redraws, and nothing runs while no data arrives (idle CPU stays near zero).

/// Cheap to clone and safe to use from any thread.
#[derive(Clone)]
pub struct Repaint(async_channel::Sender<()>);

impl Repaint {
    /// The handle, and the receiving end the app waits on.
    pub fn channel() -> (Repaint, async_channel::Receiver<()>) {
        let (tx, rx) = async_channel::bounded(1);
        (Repaint(tx), rx)
    }

    /// A handle nothing listens to (tests, and platforms without a tray).
    #[cfg(test)]
    pub fn detached() -> Repaint {
        Repaint::channel().0
    }

    /// Ask the UI to redraw soon. Never blocks.
    pub fn request_repaint(&self) {
        // Full means a redraw is already pending; closed means the app is shutting down.
        let _ = self.0.try_send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_merged_and_never_block() {
        let (repaint, rx) = Repaint::channel();
        for _ in 0..5 {
            repaint.request_repaint();
        }
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err(), "five requests wake the UI once");
        Repaint::detached().request_repaint();
    }
}
