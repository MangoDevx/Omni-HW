//! NVMe SMART / health log through the kernel's admin passthrough (`NVME_IOCTL_ADMIN_CMD` on
//! `/dev/nvmeN`). Opening the controller needs root, so this returns `None` otherwise.

use crate::storage::NvmeHealth;

/// `struct nvme_admin_cmd` from `<linux/nvme_ioctl.h>`.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct AdminCmd {
    pub opcode: u8,
    pub flags: u8,
    pub rsvd1: u16,
    pub nsid: u32,
    pub cdw2: u32,
    pub cdw3: u32,
    pub metadata: u64,
    pub addr: u64,
    pub metadata_len: u32,
    pub data_len: u32,
    pub cdw10: u32,
    pub cdw11: u32,
    pub cdw12: u32,
    pub cdw13: u32,
    pub cdw14: u32,
    pub cdw15: u32,
    pub timeout_ms: u32,
    pub result: u32,
}

/// `_IOWR('N', 0x41, struct nvme_admin_cmd)`.
pub const NVME_IOCTL_ADMIN_CMD: u32 = 0xC048_4E41;
const GET_LOG_PAGE: u8 = 0x02;
const LOG_SMART: u32 = 0x02;
pub const SMART_LOG_LEN: u32 = 512;

/// Get Log Page for the controller-wide SMART / health log into `addr`.
pub fn smart_log_cmd(addr: u64) -> AdminCmd {
    // Number of dwords, zero-based, in the upper half of CDW10.
    let numd = SMART_LOG_LEN / 4 - 1;
    AdminCmd {
        opcode: GET_LOG_PAGE,
        nsid: 0xFFFF_FFFF,
        addr,
        data_len: SMART_LOG_LEN,
        cdw10: (numd << 16) | LOG_SMART,
        ..Default::default()
    }
}

/// Controller of a namespace block device: "nvme0n1" -> "nvme0".
pub fn controller_of(block: &str) -> Option<&str> {
    let rest = block.strip_prefix("nvme")?;
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    (digits > 0).then(|| &block[..4 + digits])
}

/// The SMART / health log of controller `ctrl` (e.g. "nvme0").
pub fn smart_log(ctrl: &str) -> Option<NvmeHealth> {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::open(format!("/dev/{ctrl}")).ok()?;
    let mut buf = vec![0u8; SMART_LOG_LEN as usize];
    let mut cmd = smart_log_cmd(buf.as_mut_ptr() as u64);
    // SAFETY: `cmd` is a valid nvme_admin_cmd whose data buffer (`buf`, SMART_LOG_LEN bytes)
    // outlives the call; the fd is an open NVMe controller.
    let rc = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            NVME_IOCTL_ADMIN_CMD as _,
            &mut cmd as *mut AdminCmd,
        )
    };
    if rc != 0 {
        return None;
    }
    crate::storage::parse_nvme_health(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        assert_eq!(std::mem::size_of::<AdminCmd>(), 72);
        let c = smart_log_cmd(0x1000);
        assert_eq!((c.opcode, c.nsid, c.data_len), (2, u32::MAX, 512));
        assert_eq!(c.cdw10, 0x007F_0002);
    }

    #[test]
    fn controllers() {
        assert_eq!(controller_of("nvme0n1"), Some("nvme0"));
        assert_eq!(controller_of("nvme12n3"), Some("nvme12"));
        assert_eq!(controller_of("sda"), None);
        assert_eq!(controller_of("nvme"), None);
    }
}
