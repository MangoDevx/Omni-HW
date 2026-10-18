//! Thread placement for the memory test's single-core read pass.

use crate::topology::LogicalId;
use windows::Win32::System::SystemInformation::GROUP_AFFINITY;
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadGroupAffinity, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};

/// Pin the calling thread to logical processor `id` so the scheduler doesn't migrate it mid-pass,
/// and with `boost` raise its priority so it isn't preempted either. False if the affinity
/// couldn't be set.
pub(crate) fn pin_current_thread(id: LogicalId, boost: bool) -> bool {
    let affinity = GROUP_AFFINITY {
        Mask: 1usize << id.number,
        Group: id.group,
        Reserved: [0; 3],
    };
    // SAFETY: GetCurrentThread returns a pseudo-handle valid for the calling thread; the
    // affinity struct outlives the call.
    unsafe {
        let thread = GetCurrentThread();
        let pinned = SetThreadGroupAffinity(thread, &affinity, None).as_bool();
        if boost {
            let _ = SetThreadPriority(thread, THREAD_PRIORITY_HIGHEST);
        }
        pinned
    }
}
