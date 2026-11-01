//! Erreurs de `bonk write` dans la fenêtre : le texte vient de la ligne de commande, toujours en anglais.
//! En français, le titre de son code le précède ; le détail reste tel quel, utile au support.

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
];

/// Le titre français d'un code. Public pour que le test du binaire compare à la liste de `bonk explain`.
#[must_use]
pub fn french_title(code: &str) -> Option<&'static str> {
    FRENCH_TITLES
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, title)| *title)
}

pub fn failure_text(lang: Lang, code: Option<&str>, message: &str) -> String {
    match (lang, code.and_then(french_title)) {
        (Lang::Fr, Some(title)) => format!("{title} ({message})"),
        _ => message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_francais_precede_le_detail_et_l_anglais_reste_tel_quel() {
        assert_eq!(
            failure_text(Lang::Fr, Some("E3005"), "bad hash"),
            "L'empreinte de l'image ne correspond pas (bad hash)"
        );
        assert_eq!(
            failure_text(Lang::En, Some("E3005"), "bad hash"),
            "bad hash"
        );
        assert_eq!(failure_text(Lang::Fr, None, "bad"), "bad");
    }
}
