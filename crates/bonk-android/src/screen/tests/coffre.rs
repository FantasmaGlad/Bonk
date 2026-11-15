use super::*;

/// The phone runs the vault through Kotlin, which opens the key; what comes back fills the vault panel,
/// never the write panel.
#[test]
fn le_coffre_passe_par_kotlin_et_ses_retours_vont_au_coffre()
-> Result<(), Box<dyn std::error::Error>> {
    let mut model = loaded();
    let message = message_of(&json!({"type": "vault", "action": "open"}), &model);
    let effects = model.update(message.ok_or("message du coffre refusé")?);
    let out = effects_out(&mut model, effects, None);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["do"], "vault");
    assert_eq!(out[0]["action"], "list");
    assert_eq!(out[0]["device"], "/dev/bus/usb/001/002");
    assert!(model.vault.is_busy());
    assert!(matches!(model.write, WriteState::Draft { .. }));
    let failed =
        json!({"type": "failed", "message": "the drive is no longer connected", "code": "E1003"});
    let _ = model.update(message_of(&failed, &model).ok_or("échec de l'application refusé")?);
    assert!(matches!(
        &model.vault,
        VaultState::Finished { outcome: Outcome::Failed { code: Some(code), .. }, .. } if code == "E1003"
    ));
    assert!(matches!(model.write, WriteState::Draft { .. }));
    assert_eq!(state(&model)["vault"]["buttons"][0]["action"], "open");
    Ok(())
}

#[test]
fn hors_ligne_le_coffre_ne_propose_que_ce_qui_reste_sur_l_appareil() {
    let actions = |model: &Model| -> Vec<String> {
        vault_state(model)["buttons"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|button| button["action"].as_str().map(str::to_owned))
            .collect()
    };
    let mut model = loaded();
    model.offline = true;
    model.vault = VaultState::Ready {
        disk_id: "usb-a".to_owned(),
        images: Vec::new(),
    };
    assert_eq!(actions(&model), ["browse", "close"]);
    model.vault = VaultState::NotVault {
        disk_id: "usb-a".to_owned(),
    };
    assert_eq!(actions(&model), ["close"]);
    model.offline = false;
    assert_eq!(actions(&model), ["prepare", "close"]);
}
