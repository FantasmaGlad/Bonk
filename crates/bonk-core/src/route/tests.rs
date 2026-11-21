use super::*;

use ImageKind::{HybridIso, PlainIso, Unknown};
use WriteMode::{Auto, Extract, Hybrid, Raw};

struct Case {
    kind: ImageKind,
    udf: bool,
    truncated: bool,
    mode: WriteMode,
    compressed: bool,
    windows: Option<&'static str>,
    hybrid: Result<&'static str, NotHybrid>,
}

const BASE: Case = Case {
    kind: PlainIso,
    udf: false,
    truncated: false,
    mode: Auto,
    compressed: false,
    windows: None,
    hybrid: Ok("mbr"),
};

type Outcome = Result<(Route<&'static str, &'static str>, Option<Refusal>), Refusal>;

fn image(kind: ImageKind, udf: bool, truncated: bool) -> ImageReport {
    ImageReport {
        kind,
        bios: kind == HybridIso,
        uefi: kind == HybridIso,
        udf,
        declared_bytes: Some(4096),
        truncated: Some(truncated),
    }
}

fn run(case: Case) -> Outcome {
    let asked = case.mode == Hybrid;
    let report = image(case.kind, case.udf, case.truncated);
    plan(&report, case.mode, case.compressed, case.windows, || {
        assert!(asked, "MBR computed outside hybrid mode");
        case.hybrid
    })
    .map(|planned| (planned.route, planned.overridden))
}

fn check(cases: Vec<(Case, Outcome)>) {
    for (index, (case, expected)) in cases.into_iter().enumerate() {
        assert_eq!(run(case), expected, "case {index}");
    }
}

#[test]
fn chaque_image_prend_la_voie_qui_la_demarre() {
    check(vec![
        (
            Case {
                kind: HybridIso,
                ..BASE
            },
            Ok((Route::Raw, None)),
        ),
        (
            Case {
                udf: true,
                windows: Some("win"),
                ..BASE
            },
            Ok((Route::Extract("win"), None)),
        ),
        (
            Case {
                mode: Hybrid,
                ..BASE
            },
            Ok((Route::Hybrid("mbr"), None)),
        ),
        (
            Case {
                kind: Unknown,
                mode: Raw,
                ..BASE
            },
            Ok((Route::Raw, Some(Refusal::NoStructure))),
        ),
    ]);
}

#[test]
fn une_image_sans_voie_est_refusee_en_disant_pourquoi() {
    let truncated = Refusal::Truncated { declared: 4096 };
    let no_efi = NotHybrid::NoEfiImage;
    check(vec![
        (BASE, Err(Refusal::NoDiskBoot { udf: false })),
        (
            Case { udf: true, ..BASE },
            Err(Refusal::NoDiskBoot { udf: true }),
        ),
        (
            Case {
                mode: Extract,
                compressed: true,
                ..BASE
            },
            Err(Refusal::ExtractNeedsPlainIso),
        ),
        (
            Case {
                kind: HybridIso,
                mode: Extract,
                ..BASE
            },
            Err(Refusal::NotWindows),
        ),
        (
            Case {
                mode: Hybrid,
                hybrid: Err(no_efi),
                ..BASE
            },
            Err(Refusal::NotHybrid(no_efi)),
        ),
        (
            Case {
                mode: Hybrid,
                truncated: true,
                ..BASE
            },
            Err(truncated.clone()),
        ),
        (
            Case {
                kind: Unknown,
                ..BASE
            },
            Err(Refusal::NoStructure),
        ),
        (
            Case {
                kind: HybridIso,
                truncated: true,
                ..BASE
            },
            Err(truncated),
        ),
    ]);
}

#[test]
fn les_fichiers_windows_ne_sont_cherches_que_dans_une_iso_lisible_qui_en_a_besoin() {
    let cases = [
        (Case { udf: true, ..BASE }, true),
        (BASE, false),
        (
            Case {
                kind: HybridIso,
                udf: true,
                ..BASE
            },
            false,
        ),
        (
            Case {
                udf: true,
                compressed: true,
                ..BASE
            },
            false,
        ),
        (
            Case {
                kind: HybridIso,
                mode: Extract,
                ..BASE
            },
            true,
        ),
        (
            Case {
                udf: true,
                mode: Extract,
                compressed: true,
                ..BASE
            },
            false,
        ),
        (
            Case {
                udf: true,
                mode: Raw,
                ..BASE
            },
            false,
        ),
        (
            Case {
                udf: true,
                mode: Hybrid,
                ..BASE
            },
            false,
        ),
    ];
    for (index, (case, expected)) in cases.into_iter().enumerate() {
        let report = image(case.kind, case.udf, case.truncated);
        assert_eq!(
            looks_for_windows(&report, case.mode, case.compressed),
            expected,
            "case {index}"
        );
    }
}
