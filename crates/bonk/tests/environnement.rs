//! Règle commune aux tests qui exigent un outil du système : absent hors CI, le test se saute en le
//! disant ; absent en CI, il échoue (un saut silencieux y ferait passer pour verte une vérification
//! jamais exécutée).

mod common;

use common::decider;

#[test]
fn un_outil_present_est_utilise_en_ci_comme_ailleurs() {
    assert!(decider("gzip", true, true));
    assert!(decider("gzip", true, false));
}

#[test]
fn un_outil_absent_hors_ci_saute_le_test() {
    assert!(!decider("xz", false, false));
}

#[test]
#[should_panic(expected = "xz")]
fn un_outil_absent_en_ci_fait_echouer_le_test() {
    decider("xz", false, true);
}
