//! Boot mode and platform security from sysfs: UEFI (`/sys/firmware/efi`), Secure Boot (the
//! `SecureBoot` EFI variable, readable by everyone) and the TPM (`/sys/class/tpm`).

use super::Root;
use crate::checkup::{Firmware, Tpm};

const EFI: &str = "/sys/firmware/efi";
/// `SecureBoot` in the EFI global variable namespace.
const SECURE_BOOT: &str =
    "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";
const TPM: &str = "/sys/class/tpm";

pub fn firmware(root: &Root) -> Firmware {
    let uefi = root.exists(EFI);
    Firmware {
        uefi: Some(uefi),
        // efivarfs files start with 4 bytes of attributes; the value follows.
        secure_boot: uefi
            .then(|| root.read_bytes(SECURE_BOOT))
            .flatten()
            .and_then(|b| b.get(4).map(|&v| v == 1)),
        tpm: tpm(root),
    }
}

fn tpm(root: &Root) -> Tpm {
    let Some(dev) = root
        .list(TPM)
        .into_iter()
        .find(|d| d.starts_with("tpm") && !d.starts_with("tpmrm"))
    else {
        return Tpm::Missing;
    };
    match root
        .read(&format!("{TPM}/{dev}/tpm_version_major"))
        .as_deref()
    {
        Some("2") => Tpm::V20,
        Some("1") => Tpm::V12,
        _ => Tpm::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::Fixture;

    #[test]
    fn secure_uefi_with_tpm2() {
        let f = Fixture::new();
        f.file(&SECURE_BOOT[1..], [6, 0, 0, 0, 1])
            .file("sys/class/tpm/tpm0/tpm_version_major", "2\n")
            .dir("sys/class/tpm/tpmrm0");
        let fw = firmware(&f.root());
        assert_eq!(fw.uefi, Some(true));
        assert_eq!(fw.secure_boot, Some(true));
        assert_eq!(fw.tpm, Tpm::V20);
    }

    #[test]
    fn legacy_boot_without_tpm() {
        let f = Fixture::new();
        f.dir("sys/class");
        let fw = firmware(&f.root());
        assert_eq!(fw.uefi, Some(false));
        assert_eq!(fw.secure_boot, None);
        assert_eq!(fw.tpm, Tpm::Missing);

        f.file(&SECURE_BOOT[1..], [6, 0, 0, 0, 0]);
        assert_eq!(firmware(&f.root()).secure_boot, Some(false));
    }
}
