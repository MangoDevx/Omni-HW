//! ATA SMART and IDENTIFY DEVICE for SATA drives, through SCSI generic `SG_IO` with an
//! ATA PASS-THROUGH (16) command (what `smartctl` does for drives behind libata). Opening
//! `/dev/sdX` for this needs root, so these return `None` otherwise.

/// `struct sg_io_hdr` from `<scsi/sg.h>`.
#[repr(C)]
#[derive(Debug)]
pub struct SgIoHdr {
    pub interface_id: i32,
    pub dxfer_direction: i32,
    pub cmd_len: u8,
    pub mx_sb_len: u8,
    pub iovec_count: u16,
    pub dxfer_len: u32,
    pub dxferp: *mut u8,
    pub cmdp: *const u8,
    pub sbp: *mut u8,
    pub timeout: u32,
    pub flags: u32,
    pub pack_id: i32,
    pub usr_ptr: *mut u8,
    pub status: u8,
    pub masked_status: u8,
    pub msg_status: u8,
    pub sb_len_wr: u8,
    pub host_status: u16,
    pub driver_status: u16,
    pub resid: i32,
    pub duration: u32,
    pub info: u32,
}

/// `SG_IO` ioctl number.
const SG_IO: u32 = 0x2285;
/// `SG_DXFER_FROM_DEV`.
const DXFER_FROM_DEV: i32 = -3;
const SECTOR: usize = 512;

const SMART: u8 = 0xB0;
const SMART_READ_DATA: u8 = 0xD0;
const IDENTIFY_DEVICE: u8 = 0xEC;

/// ATA PASS-THROUGH (16) reading one 512-byte sector with PIO data-in.
pub fn pass_through_cdb(command: u8, features: u8, lba_mid: u8, lba_high: u8) -> [u8; 16] {
    [
        0x85,
        4 << 1, // protocol: PIO data-in
        // T_DIR = from device, BYT_BLOK = count in blocks, T_LENGTH = in the count field.
        0b0000_1110,
        0,
        features,
        0,
        1, // one sector
        0,
        0,
        0,
        lba_mid,
        0,
        lba_high,
        0,
        command,
        0,
    ]
}

/// SMART READ DATA: the 512-byte attribute table (see [`crate::smart::parse_ata_smart`]).
pub fn smart_data(block: &str) -> Option<Vec<u8>> {
    // The SMART "key" in the LBA mid/high registers.
    read_sector(block, &pass_through_cdb(SMART, SMART_READ_DATA, 0x4F, 0xC2))
}

/// IDENTIFY DEVICE: 256 words of drive capabilities (see
/// [`crate::storage::parse_sata_link`]).
pub fn identify(block: &str) -> Option<Vec<u8>> {
    read_sector(block, &pass_through_cdb(IDENTIFY_DEVICE, 0, 0, 0))
}

fn read_sector(block: &str, cdb: &[u8; 16]) -> Option<Vec<u8>> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(format!("/dev/{block}"))
        .ok()?;
    let mut data = vec![0u8; SECTOR];
    let mut sense = [0u8; 32];
    let mut hdr = SgIoHdr {
        interface_id: i32::from(b'S'),
        dxfer_direction: DXFER_FROM_DEV,
        cmd_len: cdb.len() as u8,
        mx_sb_len: sense.len() as u8,
        iovec_count: 0,
        dxfer_len: SECTOR as u32,
        dxferp: data.as_mut_ptr(),
        cmdp: cdb.as_ptr(),
        sbp: sense.as_mut_ptr(),
        timeout: 5000,
        flags: 0,
        pack_id: 0,
        usr_ptr: std::ptr::null_mut(),
        status: 0,
        masked_status: 0,
        msg_status: 0,
        sb_len_wr: 0,
        host_status: 0,
        driver_status: 0,
        resid: 0,
        duration: 0,
        info: 0,
    };
    // SAFETY: `hdr` is a valid sg_io_hdr; its data, command and sense buffers (`data`, `cdb`,
    // `sense`, with the lengths given) all outlive the call, and the fd is an open block device.
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), SG_IO as _, &mut hdr as *mut SgIoHdr) };
    let ok = rc == 0 && hdr.status == 0 && hdr.host_status == 0 && hdr.driver_status == 0;
    // A drive that doesn't understand the command returns all zeros.
    (ok && data.iter().any(|&b| b != 0)).then_some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        assert_eq!(std::mem::size_of::<SgIoHdr>(), 88);
    }

    #[test]
    fn smart_cdb() {
        let c = pass_through_cdb(SMART, SMART_READ_DATA, 0x4F, 0xC2);
        assert_eq!(c[0], 0x85);
        assert_eq!(c[1], 0x08);
        assert_eq!(c[2], 0x0E);
        assert_eq!(
            (c[4], c[6], c[10], c[12], c[14]),
            (0xD0, 1, 0x4F, 0xC2, 0xB0)
        );
    }
}
