//! Owned device handles and `DeviceIoControl`, shared by the storage and battery collectors.

use std::ptr;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::{HSTRING, Owned};

/// A device handle opened with `CreateFileW`, closed on drop.
pub(crate) struct Handle(Owned<HANDLE>);

// SAFETY: a file/device handle is a process-wide kernel object reference that any thread may
// use; the sensor sampler moves it to its own thread and only uses it there.
unsafe impl Send for Handle {}

impl Handle {
    /// Open an existing file or device (`\\.\PhysicalDrive0`, `\\.\C:`, a device interface
    /// path) with the given access rights; 0 asks for metadata/IOCTL access only.
    pub(crate) fn open(path: &str, access: u32) -> Option<Handle> {
        Self::try_open(path, access).ok()
    }

    /// [`Handle::open`] keeping the error, so callers can tell "missing" from "access denied".
    pub(crate) fn try_open(path: &str, access: u32) -> windows::core::Result<Handle> {
        // SAFETY: the path is a NUL-terminated HSTRING that outlives the call; no security
        // attributes or template handle.
        let h = unsafe {
            CreateFileW(
                &HSTRING::from(path),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }?;
        // SAFETY: CreateFileW succeeded, so `h` is a fresh handle owned by nobody else.
        Ok(Handle(unsafe { Owned::new(h) }))
    }

    /// Synchronous `DeviceIoControl` with `input` sent as-is; returns the number of bytes
    /// written to `out`.
    pub(crate) fn ioctl<I: ?Sized>(&self, code: u32, input: &I, out: &mut [u8]) -> Option<usize> {
        self.try_ioctl(code, input, out).ok()
    }

    /// [`Handle::ioctl`] keeping the error.
    pub(crate) fn try_ioctl<I: ?Sized>(
        &self,
        code: u32,
        input: &I,
        out: &mut [u8],
    ) -> windows::core::Result<usize> {
        let mut got = 0;
        // SAFETY: the input pointer/size describe `input` and the output pointer/size describe
        // `out`, both live for the (synchronous, non-overlapped) call.
        unsafe {
            DeviceIoControl(
                *self.0,
                code,
                Some(ptr::from_ref(input).cast()),
                size_of_val(input) as u32,
                Some(out.as_mut_ptr().cast()),
                out.len() as u32,
                Some(&mut got),
                None,
            )
        }?;
        Ok(got as usize)
    }
}

/// A Win32 struct copied from the start of an IOCTL output buffer; `None` if it is too short.
///
/// # Safety
///
/// Every bit pattern must be a valid `T` (plain-data Win32 structs of integers, newtype
/// enums and GUIDs qualify).
pub(crate) unsafe fn read_struct<T: Copy>(b: &[u8]) -> Option<T> {
    // SAFETY: the length is checked and the bytes are initialized; `read_unaligned` copes with
    // any alignment, and the caller guarantees any bit pattern is a valid `T`.
    (b.len() >= size_of::<T>()).then(|| unsafe { ptr::read_unaligned(b.as_ptr().cast()) })
}
