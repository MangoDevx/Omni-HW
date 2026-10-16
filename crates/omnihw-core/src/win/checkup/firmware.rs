//! Boot mode (UEFI or legacy BIOS), Secure Boot and TPM.

use crate::checkup::{Firmware, Tpm};
use crate::win::registry::{self, Hive};
use windows::Win32::Foundation::TBS_E_TPM_NOT_FOUND;
use windows::Win32::System::SystemInformation::{
    FIRMWARE_TYPE, FirmwareTypeBios, FirmwareTypeUefi, GetFirmwareType,
};
use windows::Win32::System::TpmBaseServices::{
    TBS_SUCCESS, TPM_DEVICE_INFO, TPM_VERSION_12, TPM_VERSION_20, Tbsi_GetDeviceInfo,
};

pub fn firmware() -> Firmware {
    let mut ft = FIRMWARE_TYPE::default();
    // SAFETY: valid out pointer to a local.
    let uefi = unsafe { GetFirmwareType(&mut ft) }
        .ok()
        .and_then(|_| match ft {
            t if t == FirmwareTypeUefi => Some(true),
            t if t == FirmwareTypeBios => Some(false),
            _ => None,
        });
    let secure_boot = registry::dword(
        Hive::LocalMachine,
        r"SYSTEM\CurrentControlSet\Control\SecureBoot\State",
        "UEFISecureBootEnabled",
    )
    .map(|v| v != 0)
    // Legacy boot can't use Secure Boot.
    .or((uefi == Some(false)).then_some(false));
    Firmware {
        uefi,
        secure_boot,
        tpm: tpm(),
    }
}

fn tpm() -> Tpm {
    let mut info = TPM_DEVICE_INFO::default();
    // SAFETY: the out buffer is a TPM_DEVICE_INFO of the declared size.
    let r = unsafe {
        Tbsi_GetDeviceInfo(
            size_of::<TPM_DEVICE_INFO>() as u32,
            (&mut info as *mut TPM_DEVICE_INFO).cast(),
        )
    };
    match r {
        TBS_SUCCESS => match info.tpmVersion {
            TPM_VERSION_20 => Tpm::V20,
            TPM_VERSION_12 => Tpm::V12,
            _ => Tpm::Unknown,
        },
        // No TPM, or it is turned off in the firmware.
        r if r == TBS_E_TPM_NOT_FOUND.0 as u32 => Tpm::Missing,
        _ => Tpm::Unknown,
    }
}
