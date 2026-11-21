use bonk_hal::Lang;

const FRENCH_TITLES: &[(&str, &str)] = &[
    ("E0001", "Valeur d'argument invalide"),
    ("E1001", "Échec de la détection des disques"),
    ("E1002", "Détection des disques indisponible sur ce système"),
    ("E1003", "Disque introuvable"),
    ("E1004", "Échec de la lecture brute du disque"),
    ("E2001", "Cible refusée par le filtre de sécurité"),
    ("E2002", "L'identité du disque a changé avant l'écriture"),
    ("E2003", "Impossible de démonter les partitions de la cible"),
    ("E2004", "Confirmation requise"),
    ("E2005", "Confirmation requise pour un disque interne"),
    ("E3001", "Image illisible"),
    ("E3002", "Format d'image non pris en charge"),
    ("E3003", "L'image est plus grande que la cible"),
    ("E3004", "Échec de la décompression"),
    ("E3005", "L'empreinte de l'image ne correspond pas"),
    ("E3006", "L'image ne démarrerait pas écrite telle quelle"),
    ("E3007", "L'image est plus courte qu'elle ne le déclare"),
    ("E3008", "Échec du téléchargement de l'image"),
    ("E3009", "Catalogue d'images indisponible ou refusé"),
    ("E4001", "Échec de l'écriture"),
    ("E4002", "Échec de la vérification après écriture"),
    ("E4003", "Opération annulée"),
    ("E4004", "Ouverture du disque refusée"),
    ("E4005", "Échec de la réparation de la GPT"),
    ("E5001", "Réglage de journal invalide"),
    ("E5002", "Configuration invalide"),
    ("E5003", "La fenêtre n'a pas pu s'ouvrir"),
    ("E5004", "Impossible d'écrire la sortie"),
    ("E5005", "Capture d'énumération invalide"),
    ("E5006", "Fichier de journal inutilisable"),
    ("E6001", "Requête de service invalide"),
    ("E7001", "Échec du formatage"),
    ("E7002", "Système de fichiers indisponible sur cet hôte"),
    ("E7003", "Aucun Bonk Vault lisible sur ce disque"),
    ("E7004", "Cette ISO ne peut pas démarrer depuis le coffre"),
    ("E7005", "Pas assez de place sur le coffre"),
    ("E7006", "Nom d'ISO déjà présent sur le coffre, ou absent"),
];

/// French title of a code. Public so the binary test compares it with `bonk explain`.
#[must_use]
pub fn french_title(code: &str) -> Option<&'static str> {
    FRENCH_TITLES
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, title)| *title)
}

#[must_use]
pub fn technical(message: &str) -> String {
    let mut letters = message.chars();
    letters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(letters).collect()
    })
}

/// The code follows the title, so the engine's message closes the line instead of splitting a sentence.
#[must_use]
pub fn failure_text(lang: Lang, code: Option<&str>, message: &str) -> String {
    let tag = code.map_or_else(String::new, |c| format!(" [{c}]"));
    match (lang, code.and_then(french_title)) {
        (Lang::En, _) => format!("FAILED: {message}{tag}"),
        (Lang::Fr, Some(title)) => format!("ÉCHEC : {title}{tag}. {}", technical(message)),
        (Lang::Fr, None) => format!("ÉCHEC{tag}. {}", technical(message)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_message_du_moteur_suit_la_phrase_sans_commentaire_sur_sa_langue() {
        assert_eq!(
            failure_text(Lang::Fr, Some("E3005"), "bad hash"),
            "ÉCHEC : L'empreinte de l'image ne correspond pas [E3005]. Bad hash"
        );
        assert_eq!(
            failure_text(Lang::En, Some("E3005"), "bad hash"),
            "FAILED: bad hash [E3005]"
        );
        assert_eq!(failure_text(Lang::Fr, None, "bad"), "ÉCHEC. Bad");
    }
}
