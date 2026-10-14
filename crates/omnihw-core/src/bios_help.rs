//! Board-aware BIOS help: which key opens the UEFI setup and where a vendor keeps common
//! settings, from the SMBIOS board (or system) manufacturer. Used to turn "enable XMP in the
//! BIOS" into concrete steps with a Google search for the exact board.

use crate::SystemInfo;
use crate::diagnostics::Step;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardVendor {
    Asus,
    Msi,
    Gigabyte,
    Asrock,
    Dell,
    Hp,
    Lenovo,
    Acer,
    Other,
}

impl BoardVendor {
    /// From an SMBIOS manufacturer string such as "ASUSTeK COMPUTER INC." or
    /// "Micro-Star International Co., Ltd.".
    pub fn from_manufacturer(m: &str) -> BoardVendor {
        let m = m.to_ascii_lowercase();
        let has = |k: &str| m.contains(k);
        if has("asustek") || m.starts_with("asus") {
            BoardVendor::Asus
        } else if has("micro-star") || m.starts_with("msi") {
            BoardVendor::Msi
        } else if has("gigabyte") {
            BoardVendor::Gigabyte
        } else if has("asrock") {
            BoardVendor::Asrock
        } else if has("dell") || has("alienware") {
            BoardVendor::Dell
        } else if has("hewlett") || m.starts_with("hp") {
            BoardVendor::Hp
        } else if has("lenovo") {
            BoardVendor::Lenovo
        } else if has("acer") {
            BoardVendor::Acer
        } else {
            BoardVendor::Other
        }
    }

    /// Short brand for search queries and text.
    pub fn name(self) -> Option<&'static str> {
        Some(match self {
            BoardVendor::Asus => "ASUS",
            BoardVendor::Msi => "MSI",
            BoardVendor::Gigabyte => "Gigabyte",
            BoardVendor::Asrock => "ASRock",
            BoardVendor::Dell => "Dell",
            BoardVendor::Hp => "HP",
            BoardVendor::Lenovo => "Lenovo",
            BoardVendor::Acer => "Acer",
            BoardVendor::Other => return None,
        })
    }

    /// Key to press right after power-on to open the UEFI setup.
    pub fn setup_key(self) -> &'static str {
        match self {
            BoardVendor::Asus | BoardVendor::Msi | BoardVendor::Gigabyte => "Del",
            BoardVendor::Asrock => "F2 or Del",
            BoardVendor::Dell => "F2",
            BoardVendor::Hp => "Esc, then F10",
            BoardVendor::Lenovo => "F1 (ThinkPad/ThinkCentre) or F2 (IdeaPad/Legion)",
            BoardVendor::Acer => "F2 or Del",
            BoardVendor::Other => "Del or F2",
        }
    }

    /// Where the vendor's UEFI keeps a setting, when known.
    pub fn menu_path(self, s: BiosSetting) -> Option<&'static str> {
        use BiosSetting::*;
        use BoardVendor::*;
        Some(match (self, s) {
            (Asus, MemoryProfile) => {
                "Ai Tweaker > Ai Overclock Tuner > XMP I / EXPO I (press F7 for Advanced Mode first)"
            }
            (Msi, MemoryProfile) => {
                "OC > Extreme Memory Profile (A-XMP / EXPO), or the XMP/EXPO switch on the EZ Mode page"
            }
            (Gigabyte, MemoryProfile) => "Tweaker > Extreme Memory Profile (X.M.P.) / EXPO",
            (Asrock, MemoryProfile) => "OC Tweaker > DRAM Configuration > Load XMP/EXPO Setting",
            (Asus, ResizableBar) => {
                "Advanced > PCI Subsystem Settings > Above 4G Decoding: Enabled, then Re-Size BAR Support: Auto/Enabled"
            }
            (Msi, ResizableBar) => {
                "Settings > Advanced > PCI Subsystem Settings > Above 4G Memory: Enabled, then Re-Size BAR Support: Enabled"
            }
            (Gigabyte, ResizableBar) => {
                "Settings > IO Ports > Above 4G Decoding: Enabled, then Re-Size BAR Support: Auto"
            }
            (Asrock, ResizableBar) => {
                "Advanced > Chipset Configuration > Above 4G Decoding: Enabled, then C.A.M. / Re-Size BAR Support: Enabled"
            }
            (Asus, SecureBoot) => {
                "Boot > Secure Boot > OS Type: Windows UEFI mode (Key Management > Install Default Secure Boot keys if needed)"
            }
            (Msi, SecureBoot) => "Settings > Security > Secure Boot > Secure Boot: Enabled",
            (Gigabyte, SecureBoot) => "Boot > Secure Boot: Enabled",
            (Asrock, SecureBoot) => "Security > Secure Boot: Enabled",
            (Dell, SecureBoot) => "Boot Configuration (or Secure Boot) > Enable Secure Boot",
            (Hp, SecureBoot) => "Advanced (or Security) > Secure Boot Configuration",
            (Lenovo, SecureBoot) => "Security > Secure Boot: Enabled",
            (Asus, Csm) => "Boot > CSM (Compatibility Support Module) > Launch CSM: Disabled",
            (Msi, Csm) => {
                "Settings > Advanced > Windows OS Configuration > BIOS UEFI/CSM Mode: UEFI"
            }
            (Gigabyte, Csm) => "Boot > CSM Support: Disabled",
            (Asrock, Csm) => "Boot > CSM (Compatibility Support Module): Disabled",
            (Asus, Tpm) => {
                "Advanced > PCH-FW Configuration > PTT: Enable (Intel), or Advanced > AMD fTPM configuration > Firmware TPM (AMD)"
            }
            (Msi, Tpm) => {
                "Settings > Security > Trusted Computing > Security Device Support: Enable (AMD fTPM / Intel PTT)"
            }
            (Gigabyte, Tpm) => {
                "Settings > Miscellaneous > Intel Platform Trust Technology (PTT) or AMD CPU fTPM: Enabled"
            }
            (Asrock, Tpm) => {
                "Security > Intel Platform Trust Technology, or Advanced > CPU Configuration > AMD fTPM switch"
            }
            (Asus, LoadDefaults) => "Exit > Load Optimized Defaults (or press F5)",
            (Msi, LoadDefaults) => "press F6 (Load Optimized Defaults)",
            (Gigabyte, LoadDefaults) => "Save & Exit > Load Optimized Defaults (or press F7)",
            (Asrock, LoadDefaults) => "Exit > Load UEFI Defaults (or press F9)",
            (Dell, LoadDefaults) => "Restore Settings > BIOS Defaults",
            (Hp, LoadDefaults) => "Main > Apply Factory Defaults and Exit (or press F9)",
            (Lenovo, LoadDefaults) => "Restart > Load Setup Defaults (or press F9)",
            (Asus, BootOrder) => "Boot > Boot Option Priorities > Boot Option #1",
            (Msi, BootOrder) => "Settings > Boot > Boot Option #1",
            (Gigabyte, BootOrder) => "Boot > Boot Option Priorities > Boot Option #1",
            (Asrock, BootOrder) => "Boot > Boot Option #1",
            (Dell, BootOrder) => "Boot Configuration > Boot Sequence",
            (Hp, BootOrder) => "Advanced > Boot Options > UEFI Boot Order",
            (Lenovo, BootOrder) => "Startup > Boot (or Boot > Boot Priority Order)",
            _ => return None,
        })
    }
}

/// UEFI settings the Health Check gives directions for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BiosSetting {
    /// XMP / EXPO memory profile.
    MemoryProfile,
    ResizableBar,
    SecureBoot,
    /// Turn off legacy boot (CSM).
    Csm,
    /// Firmware TPM (fTPM / PTT).
    Tpm,
    /// Load optimized defaults (undo overclocks).
    LoadDefaults,
    /// Which drive the PC starts from.
    BootOrder,
}

/// The motherboard (desktops) or system (laptops/OEM PCs) the BIOS belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    pub vendor: BoardVendor,
    /// e.g. "ROG STRIX B650E-F GAMING WIFI"; empty if unknown.
    pub model: String,
}

/// Placeholder strings OEMs leave in SMBIOS.
fn is_placeholder(s: &str) -> bool {
    let l = s.trim().to_ascii_lowercase();
    l.is_empty()
        || l.contains("to be filled")
        || l.contains("default string")
        || l == "system product name"
        || l == "not applicable"
}

impl Board {
    pub fn detect(info: &SystemInfo) -> Board {
        let sm = info.smbios.as_ref();
        let board = sm.and_then(|s| s.board.as_ref());
        let system = sm.and_then(|s| s.system.as_ref());
        let manufacturer = board
            .map(|b| b.manufacturer.as_str())
            .filter(|m| !is_placeholder(m))
            .or(system.map(|s| s.manufacturer.as_str()))
            .unwrap_or("");
        let vendor = BoardVendor::from_manufacturer(manufacturer);
        // OEM boards have cryptic names ("0X8RDT"); the system product is what people search for.
        let oem = matches!(
            vendor,
            BoardVendor::Dell | BoardVendor::Hp | BoardVendor::Lenovo | BoardVendor::Acer
        );
        let board_model = board
            .map(|b| b.product.as_str())
            .filter(|p| !is_placeholder(p));
        let system_model = system
            .map(|s| s.product.as_str())
            .filter(|p| !is_placeholder(p));
        let model = if oem {
            system_model.or(board_model)
        } else {
            board_model.or(system_model)
        };
        Board {
            vendor,
            model: model.unwrap_or("").trim().to_string(),
        }
    }

    /// "ASUS ROG STRIX B650E-F GAMING WIFI": vendor plus model, without repeating the vendor.
    pub fn search_name(&self) -> String {
        let model = self.model.as_str();
        match self.vendor.name() {
            Some(v) if !model.to_ascii_lowercase().contains(&v.to_ascii_lowercase()) => {
                format!("{v} {model}").trim().to_string()
            }
            _ => model.to_string(),
        }
    }

    /// "your ASUS board" / "your motherboard".
    fn noun(&self) -> String {
        match self.vendor.name() {
            Some(v) => format!("your {v} board"),
            None => "your motherboard".into(),
        }
    }
}

fn setting_query(s: BiosSetting, profile: &str) -> String {
    match s {
        BiosSetting::MemoryProfile => format!("enable {profile}"),
        BiosSetting::ResizableBar => "enable Resizable BAR".into(),
        BiosSetting::SecureBoot => "enable Secure Boot".into(),
        BiosSetting::Csm => "disable CSM UEFI boot".into(),
        BiosSetting::Tpm => "enable TPM 2.0 fTPM".into(),
        BiosSetting::LoadDefaults => "load optimized defaults BIOS".into(),
        BiosSetting::BootOrder => "change boot order BIOS".into(),
    }
}

/// Step: how to get into the UEFI setup on this board.
pub fn enter_setup_step(board: &Board, uefi: bool) -> Step {
    let restart = if uefi {
        " Or use the Restart into UEFI / BIOS button below."
    } else {
        ""
    };
    Step::new(format!(
        "Restart and press {} repeatedly as the PC starts to open the UEFI/BIOS setup on {}.{restart}",
        board.vendor.setup_key(),
        board.noun()
    ))
}

/// Step: where the setting lives, with a Google search for this exact board.
/// `profile` names the memory profile ("XMP" / "EXPO") for [`BiosSetting::MemoryProfile`].
pub fn setting_step(board: &Board, s: BiosSetting, generic: &str, profile: &str) -> Step {
    let text = match board.vendor.menu_path(s) {
        Some(path) => format!("{generic} On {}: {path}.", board.noun()),
        None => format!(
            "{generic} Menu names vary by vendor; the search below finds the steps for your model."
        ),
    };
    let name = board.search_name();
    let query = if name.is_empty() {
        format!("motherboard BIOS {}", setting_query(s, profile))
    } else {
        format!("{name} {}", setting_query(s, profile))
    };
    Step::search(text, query)
}

/// Step: save and leave the setup.
pub fn save_step() -> Step {
    Step::new("Save and exit (usually F10). The PC restarts into Windows.")
}

/// The usual three steps for changing one UEFI setting on this PC.
pub fn bios_steps(info: &SystemInfo, s: BiosSetting, generic: &str, profile: &str) -> Vec<Step> {
    let board = Board::detect(info);
    let uefi = info.checkup.firmware.uefi == Some(true);
    vec![
        enter_setup_step(&board, uefi),
        setting_step(&board, s, generic, profile),
        save_step(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Link;
    use crate::smbios::{Board as SmBoard, Smbios, SystemInfo as SmSystem};

    fn info(board_mfr: &str, board: &str, sys_mfr: &str, sys: &str) -> SystemInfo {
        SystemInfo {
            smbios: Some(Smbios {
                board: Some(SmBoard {
                    manufacturer: board_mfr.into(),
                    product: board.into(),
                    ..Default::default()
                }),
                system: Some(SmSystem {
                    manufacturer: sys_mfr.into(),
                    product: sys.into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn vendors() {
        use BoardVendor::*;
        let v = BoardVendor::from_manufacturer;
        assert_eq!(v("ASUSTeK COMPUTER INC."), Asus);
        assert_eq!(v("Micro-Star International Co., Ltd."), Msi);
        assert_eq!(v("Gigabyte Technology Co., Ltd."), Gigabyte);
        assert_eq!(v("ASRock"), Asrock);
        assert_eq!(v("Dell Inc."), Dell);
        assert_eq!(v("HP"), Hp);
        assert_eq!(v("LENOVO"), Lenovo);
        assert_eq!(v("Some Board Co"), Other);
        assert_eq!(Asus.setup_key(), "Del");
        assert!(Msi.menu_path(BiosSetting::MemoryProfile).is_some());
        assert!(Other.menu_path(BiosSetting::MemoryProfile).is_none());
    }

    #[test]
    fn board_detection_and_search() {
        let b = Board::detect(&info(
            "ASUSTeK COMPUTER INC.",
            "ROG STRIX B650E-F GAMING WIFI",
            "System manufacturer",
            "System Product Name",
        ));
        assert_eq!(b.vendor, BoardVendor::Asus);
        assert_eq!(b.search_name(), "ASUS ROG STRIX B650E-F GAMING WIFI");
        let s = setting_step(&b, BiosSetting::MemoryProfile, "Enable EXPO.", "EXPO");
        assert!(s.text.contains("Ai Overclock Tuner"));
        assert_eq!(
            s.link,
            Some(Link::Search(
                "ASUS ROG STRIX B650E-F GAMING WIFI enable EXPO".into()
            ))
        );

        let dell = Board::detect(&info("Dell Inc.", "0X8RDT", "Dell Inc.", "XPS 8960"));
        assert_eq!(dell.search_name(), "Dell XPS 8960");

        let msi = Board::detect(&info(
            "Micro-Star International Co., Ltd.",
            "MAG B650 TOMAHAWK WIFI (MS-7D75)",
            "",
            "",
        ));
        assert_eq!(msi.search_name(), "MSI MAG B650 TOMAHAWK WIFI (MS-7D75)");

        let unknown = Board::detect(&SystemInfo::default());
        let s = setting_step(&unknown, BiosSetting::SecureBoot, "Enable Secure Boot.", "");
        assert_eq!(
            s.link,
            Some(Link::Search("motherboard BIOS enable Secure Boot".into()))
        );
        assert!(
            enter_setup_step(&unknown, true)
                .text
                .contains("Restart into UEFI")
        );
        assert!(
            !enter_setup_step(&unknown, false)
                .text
                .contains("Restart into UEFI")
        );
    }
}
