use crate::bytes::{get_u16, get_u32};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MACHINE_X86_64: u16 = 0x8664;
const MACHINE_X86: u16 = 0x14C;
const MACHINE_ARM64: u16 = 0xAA64;
const OPTIONAL_PE32: u16 = 0x10B;
const OPTIONAL_PE32_PLUS: u16 = 0x20B;
/// `commonName` in DER: OID 2.5.4.3.
const COMMON_NAME_OID: [u8; 5] = [0x06, 0x03, 0x55, 0x04, 0x03];
pub const PCA_2011: &str = "Microsoft Windows Production PCA 2011";
pub const UEFI_CA_2023: &str = "Windows UEFI CA 2023";
pub const THIRD_PARTY_CAS: [&str; 2] = [
    "Microsoft Corporation UEFI CA 2011",
    "Microsoft UEFI CA 2023",
];

/// Processor a UEFI loader runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Machine {
    #[serde(rename = "x86_64")]
    X86_64,
    #[serde(rename = "x86")]
    X86,
    #[serde(rename = "arm64")]
    Arm64,
}

impl Machine {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::X86 => "x86",
            Self::Arm64 => "arm64",
        }
    }
}

/// Authority of the loader certificate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Signer {
    /// Old certificate: refused by a PC that revoked it.
    WindowsProductionPca2011,
    /// New certificate: refused by a PC whose firmware does not know it yet.
    WindowsUefiCa2023,
    /// Microsoft's third-party authority, which signs shim and other vendors' loaders.
    MicrosoftUefiCa(Vec<String>),
    /// Another authority, named.
    Other(String),
    /// No signature: refused with Secure Boot active.
    Unsigned,
}

impl Signer {
    #[must_use]
    pub fn meaning(&self) -> String {
        match self {
            Self::WindowsProductionPca2011 => format!(
                "loader signed by {PCA_2011}: with Secure Boot, refused by a PC that revoked this certificate"
            ),
            Self::WindowsUefiCa2023 => format!(
                "loader signed by {UEFI_CA_2023}: with Secure Boot, refused by a PC whose firmware does not know this certificate"
            ),
            Self::MicrosoftUefiCa(names) => format!(
                "loader signed by {}, Microsoft's third-party authority: with Secure Boot, accepted by a PC that trusts it, which some PCs turn off by default",
                names.join(" and ")
            ),
            Self::Other(name) => {
                format!("loader signed by {name}: with Secure Boot, depends on what the PC accepts")
            }
            Self::Unsigned => "unsigned loader: refused with Secure Boot active".to_owned(),
        }
    }
}

fn coff_offset(file: &[u8]) -> Option<usize> {
    if file.get(..2)? != b"MZ" {
        return None;
    }
    let pe = get_u32(file, 0x3C)? as usize;
    (file.get(pe..pe.checked_add(4)?)? == b"PE\0\0").then_some(pe + 4)
}

/// `None`: not a PE, or a machine a PC does not boot.
#[must_use]
pub fn machine(file: &[u8]) -> Option<Machine> {
    match get_u16(file, coff_offset(file)?)? {
        MACHINE_X86_64 => Some(Machine::X86_64),
        MACHINE_X86 => Some(Machine::X86),
        MACHINE_ARM64 => Some(Machine::Arm64),
        _ => None,
    }
}

fn common_names(der: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut at = 0;
    while let Some(found) = der.get(at..).and_then(|rest| {
        rest.windows(COMMON_NAME_OID.len())
            .position(|w| w == COMMON_NAME_OID)
    }) {
        let value = at + found + COMMON_NAME_OID.len();
        at = value;
        let Some(&len) = der.get(value + 1) else {
            break;
        };
        if len < 0x80
            && let Some(bytes) = der.get(value + 2..value + 2 + usize::from(len))
            && let Ok(name) = std::str::from_utf8(bytes)
        {
            names.push(name.to_owned());
        }
    }
    names
}

#[must_use]
pub fn signer(file: &[u8]) -> Option<Signer> {
    let coff = coff_offset(file)?;
    let optional = coff + 20;
    let directories = match get_u16(file, optional)? {
        OPTIONAL_PE32_PLUS => optional + 112,
        OPTIONAL_PE32 => optional + 96,
        _ => return None,
    };
    let security = directories + 4 * 8;
    let start = get_u32(file, security)? as usize;
    let len = get_u32(file, security + 4)? as usize;
    if start == 0 || len == 0 {
        return Some(Signer::Unsigned);
    }
    let table = file.get(start..start.checked_add(len)?)?;
    let names = common_names(table);
    let third_party: Vec<String> = THIRD_PARTY_CAS
        .into_iter()
        .filter(|ca| names.iter().any(|name| name == ca))
        .map(str::to_owned)
        .collect();
    if names.iter().any(|name| name == UEFI_CA_2023) {
        Some(Signer::WindowsUefiCa2023)
    } else if names.iter().any(|name| name == PCA_2011) {
        Some(Signer::WindowsProductionPca2011)
    } else if !third_party.is_empty() {
        Some(Signer::MicrosoftUefiCa(third_party))
    } else {
        Some(
            names
                .into_iter()
                .next()
                .map_or(Signer::Unsigned, Signer::Other),
        )
    }
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    use super::testing::{name, pe};

    #[test]
    fn le_certificat_du_chargeur_est_nomme() {
        let both = [name("Microsoft Corporation UEFI"), name(UEFI_CA_2023)].concat();
        assert_eq!(
            signer(&pe(MACHINE_X86_64, &both)),
            Some(Signer::WindowsUefiCa2023)
        );
        assert_eq!(
            signer(&pe(MACHINE_X86_64, &name(PCA_2011))),
            Some(Signer::WindowsProductionPca2011)
        );
        assert_eq!(
            signer(&pe(MACHINE_X86_64, &name("Autre"))),
            Some(Signer::Other("Autre".to_owned()))
        );
        assert_eq!(signer(&pe(MACHINE_X86_64, &[])), Some(Signer::Unsigned));
        assert_eq!(signer(b"pas un exe"), None);
        assert!(signer(&pe(MACHINE_X86_64, &[])).is_some_and(|s| s.meaning().contains("unsigned")));
    }

    // As Debian 13's shim: Microsoft's third-party authority of 2011 and 2023, its signer and timestamping.
    #[test]
    fn un_shim_signe_par_l_autorite_tierce_de_microsoft_est_reconnu() {
        let shim = [
            name(THIRD_PARTY_CAS[0]),
            name("Microsoft Windows UEFI Driver Publisher"),
            name("Microsoft Time-Stamp PCA 2010"),
            name(THIRD_PARTY_CAS[1]),
        ]
        .concat();
        let both = THIRD_PARTY_CAS.map(str::to_owned).to_vec();
        assert_eq!(
            signer(&pe(MACHINE_X86_64, &shim)),
            Some(Signer::MicrosoftUefiCa(both))
        );
        let old = signer(&pe(MACHINE_X86_64, &name(THIRD_PARTY_CAS[0])));
        assert_eq!(
            old,
            Some(Signer::MicrosoftUefiCa(vec![THIRD_PARTY_CAS[0].to_owned()]))
        );
        assert!(old.is_some_and(|s| s.meaning().contains("third-party")));
    }

    #[test]
    fn l_architecture_est_lue() {
        assert_eq!(machine(&pe(0x8664, &[])), Some(Machine::X86_64));
        assert_eq!(machine(&pe(0x14C, &[])), Some(Machine::X86));
        assert_eq!(machine(&pe(0xAA64, &[])), Some(Machine::Arm64));
        assert_eq!(machine(&pe(0x1C4, &[])), None);
        assert_eq!(machine(&[]), None);
    }

    #[test]
    fn un_fichier_abime_ne_fait_jamais_paniquer() {
        let good = pe(MACHINE_X86_64, &name(PCA_2011));
        for at in 0..good.len() {
            for byte in [0x00, 0xFF, 0x7F] {
                let mut bad = good.clone();
                bad[at] = byte;
                let _ = signer(&bad);
                let _ = machine(&bad);
            }
            let _ = signer(&good[..at]);
        }
    }
}
