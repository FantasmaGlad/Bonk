use super::*;

// MSWIM header naming one resource table of one entry, whose resource is solid (an install.esd).
fn solid_wim() -> Vec<u8> {
    let mut wim = vec![0u8; 208 + 50];
    wim[..8].copy_from_slice(b"MSWIM\0\0\0");
    wim[40..42].copy_from_slice(&1u16.to_le_bytes());
    wim[42..44].copy_from_slice(&1u16.to_le_bytes());
    wim[48..56].copy_from_slice(&50u64.to_le_bytes());
    wim[56..64].copy_from_slice(&208u64.to_le_bytes());
    wim[64..72].copy_from_slice(&50u64.to_le_bytes());
    wim[208 + 7] = 0x10;
    wim
}

#[test]
fn bonk_info_d_une_iso_windows_a_install_esd_solide_dit_le_refus_sans_echouer() {
    use bonk_core::pe::testing::{X86_64, pe};
    let loader = pe(X86_64, &[]);
    let wim = solid_wim();
    let iso = bonk_core::udf::testing::build_declared(&[
        ("efi/boot/bootx64.efi", &loader, loader.len() as u64),
        ("sources/install.esd", &wim, 5 << 30),
    ]);
    let source = TempSource::from_bytes("windows-esd-solide", &iso);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&["info", &source.arg()]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(
        out.contains("Route: refused, WIM with solid resources")
            && out.contains("Microsoft's ISO of this Windows"),
        "{out}"
    );
    let (_, json, _) = scenario.run(&["info", &source.arg(), "--json"]);
    let report: serde_json::Value = serde_json::from_str(&json).expect("JSON");
    assert_eq!(report["route"], serde_json::Value::Null);
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(
        err.contains("[E3002]") && err.contains("under 4 GiB"),
        "{err}"
    );
    assert!(scenario.log().is_empty(), "aucun disque touché");
}
