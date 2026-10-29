//! Codes d'erreur stables et codes de sortie (CDC §9.1).
//!
//! Chaque erreur affichée se termine par son code entre crochets (`[E1003]`) : un outil le retrouve
//! sans interpréter le texte, et `bonk explain <code>` en donne le sens, les causes probables et le
//! remède. Les codes ne sont jamais réutilisés ni renumérotés.

use std::fmt::Display;
use std::io::{self, Write};

use bonk_hal::EnumerateError;
use clap::Args;
use serde::Serialize;

/// Code de sortie d'une erreur d'exécution générale.
pub(crate) const EXIT_RUNTIME_ERROR: u8 = 1;
/// Code de sortie d'une erreur d'usage, celui de clap.
pub(crate) const EXIT_USAGE: u8 = 2;
/// Cible refusée par le filtre de sécurité, ou dont l'identité a changé.
pub(crate) const EXIT_TARGET_REFUSED: u8 = 3;
/// La vérification après écriture a échoué.
pub(crate) const EXIT_VERIFY_FAILED: u8 = 4;
/// Opération annulée par l'utilisateur.
pub(crate) const EXIT_CANCELLED: u8 = 5;
/// Source (image) illisible, de format inconnu ou trop grande.
pub(crate) const EXIT_SOURCE: u8 = 6;

/// Une erreur connue de Bonk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Code {
    InvalidArgument,
    EnumerationFailed,
    EnumerationUnsupported,
    DiskNotFound,
    RawDiskUnavailable,
    TargetRefused,
    IdentityChanged,
    UnmountFailed,
    ConfirmationRequired,
    SourceUnreadable,
    SourceFormatUnsupported,
    SourceTooLarge,
    DecompressionFailed,
    WriteFailed,
    VerifyMismatch,
    Cancelled,
    DeviceOpenDenied,
    GptRepairFailed,
    LogSettingInvalid,
    ConfigInvalid,
    WindowUnavailable,
    OutputFailed,
    FixtureInvalid,
    LogUnusable,
    ProtocolError,
}

/// Fiche d'un code, telle que la montre `bonk explain`.
pub(crate) struct Entry {
    pub(crate) code: Code,
    pub(crate) id: &'static str,
    pub(crate) exit: u8,
    pub(crate) title: &'static str,
    pub(crate) causes: &'static [&'static str],
    pub(crate) remedy: &'static str,
}

/// Le catalogue. Ordre = ordre des numéros.
pub(crate) const CATALOG: &[Entry] = &[
    Entry {
        code: Code::InvalidArgument,
        id: "E0001",
        exit: EXIT_USAGE,
        title: "Valeur d'argument invalide",
        causes: &["une valeur numérique, un décalage ou une longueur hors des limites acceptées"],
        remedy: "Relire l'aide de la commande (`bonk <commande> --help`) et corriger la valeur.",
    },
    Entry {
        code: Code::EnumerationFailed,
        id: "E1001",
        exit: EXIT_RUNTIME_ERROR,
        title: "Détection des disques impossible",
        causes: &[
            "une source système est illisible (points de montage, base udev, liste des disques)",
            "base udev absente (conteneur minimal, système sans udev)",
        ],
        remedy: "Lancer `bonk debug sources` pour voir quelle lecture échoue, puis `bonk diagnose` pour un rapport complet. Bonk refuse de rien proposer plutôt que de deviner.",
    },
    Entry {
        code: Code::EnumerationUnsupported,
        id: "E1002",
        exit: EXIT_RUNTIME_ERROR,
        title: "Détection des disques non disponible sur ce système",
        causes: &["l'énumération n'existe que sous Linux dans cette version"],
        remedy: "Utiliser un hôte Linux, ou rejouer une capture Linux avec `--fixture`.",
    },
    Entry {
        code: Code::DiskNotFound,
        id: "E1003",
        exit: EXIT_RUNTIME_ERROR,
        title: "Disque introuvable",
        causes: &[
            "l'identifiant ou le chemin ne correspond à aucun disque détecté",
            "le disque a été débranché, ou son identifiant a changé",
        ],
        remedy: "Lister les identifiants avec `bonk list --all` et réessayer.",
    },
    Entry {
        code: Code::RawDiskUnavailable,
        id: "E1004",
        exit: EXIT_RUNTIME_ERROR,
        title: "Lecture brute d'un disque réel non disponible",
        causes: &["`debug layout` et `debug hexdump` n'ouvrent que des fichiers image"],
        remedy: "Copier le début du disque dans une image avec un outil du système, ou attendre l'accès brut via les privilèges (décision D-03).",
    },
    Entry {
        code: Code::TargetRefused,
        id: "E2001",
        exit: EXIT_TARGET_REFUSED,
        title: "Cible refusée par le filtre de sécurité",
        causes: &[
            "le disque porte le système, l'ESP active, un espace d'échange ou un montage hors des chemins utilisateur",
            "le disque est membre d'un RAID, LVM, ZFS ou porte un volume empilé",
            "le disque est interne ou virtuel et l'option correspondante n'est pas active",
        ],
        remedy: "Voir les motifs avec `bonk debug filter <disque>`. Un disque interdit ne peut jamais être écrit ; démonter un montage manuel ou choisir un autre disque.",
    },
    Entry {
        code: Code::IdentityChanged,
        id: "E2002",
        exit: EXIT_TARGET_REFUSED,
        title: "L'identité du disque a changé avant l'écriture",
        causes: &[
            "le disque a été débranché puis un autre branché au même emplacement",
            "le numéro de série ou la taille ne correspondent plus",
        ],
        remedy: "Relister les disques avec `bonk list`, vérifier la cible et relancer.",
    },
    Entry {
        code: Code::UnmountFailed,
        id: "E2003",
        exit: EXIT_RUNTIME_ERROR,
        title: "Démontage des partitions de la cible impossible",
        causes: &[
            "un programme utilise encore le volume",
            "droits insuffisants pour démonter",
        ],
        remedy: "Fermer les programmes qui utilisent la clé, la démonter à la main, puis relancer.",
    },
    Entry {
        code: Code::ConfirmationRequired,
        id: "E2004",
        exit: EXIT_USAGE,
        title: "Confirmation requise",
        causes: &["une écriture sans `--yes` : Bonk ne pose jamais de question hors terminal"],
        remedy: "Décrire d'abord l'opération avec `--dry-run`, puis relancer avec `--yes`.",
    },
    Entry {
        code: Code::SourceUnreadable,
        id: "E3001",
        exit: EXIT_SOURCE,
        title: "Image illisible",
        causes: &["fichier absent, droits insuffisants, ou ce n'est pas un fichier ordinaire"],
        remedy: "Vérifier le chemin et les droits du fichier.",
    },
    Entry {
        code: Code::SourceFormatUnsupported,
        id: "E3002",
        exit: EXIT_SOURCE,
        title: "Format d'image non pris en charge",
        causes: &["format compressé reconnu mais non pris en charge (bzip2, zip...)"],
        remedy: "Décompresser l'image avec un outil du système, ou la fournir en .img, .iso, .gz, .xz ou .zst.",
    },
    Entry {
        code: Code::SourceTooLarge,
        id: "E3003",
        exit: EXIT_SOURCE,
        title: "L'image est plus grande que la cible",
        causes: &["la taille de l'image décompressée dépasse celle du disque ou de l'image cible"],
        remedy: "Choisir un support plus grand. Rien n'est écrit au-delà de la fin du support.",
    },
    Entry {
        code: Code::DecompressionFailed,
        id: "E3004",
        exit: EXIT_SOURCE,
        title: "Décompression en échec",
        causes: &["fichier compressé tronqué ou corrompu"],
        remedy: "Retélécharger l'image et vérifier son empreinte avant de recommencer.",
    },
    Entry {
        code: Code::WriteFailed,
        id: "E4001",
        exit: EXIT_RUNTIME_ERROR,
        title: "Écriture en échec",
        causes: &[
            "erreur d'entrée-sortie du support (clé défectueuse, débranchée en cours d'écriture)",
        ],
        remedy: "Le support est dans un état indéterminé : le réécrire entièrement, sur un autre support si l'erreur revient.",
    },
    Entry {
        code: Code::VerifyMismatch,
        id: "E4002",
        exit: EXIT_VERIFY_FAILED,
        title: "La vérification après écriture a échoué",
        causes: &[
            "les octets relus ne correspondent pas à l'image : support défectueux ou écriture partielle",
        ],
        remedy: "Réécrire l'image ; si l'erreur persiste, remplacer le support.",
    },
    Entry {
        code: Code::Cancelled,
        id: "E4003",
        exit: EXIT_CANCELLED,
        title: "Opération annulée",
        causes: &["annulation demandée (Ctrl+C ou requête d'annulation)"],
        remedy: "Le support est partiellement écrit et n'est pas amorçable : relancer l'écriture complète.",
    },
    Entry {
        code: Code::DeviceOpenDenied,
        id: "E4004",
        exit: EXIT_RUNTIME_ERROR,
        title: "Ouverture du disque refusée",
        causes: &[
            "droits insuffisants : l'écriture brute exige des privilèges (l'interface ne tourne jamais en administrateur)",
        ],
        remedy: "Lancer la ligne de commande avec les droits nécessaires (par exemple `sudo bonk ...`), ou attendre l'ouverture par le service du système (décision D-03).",
    },
    Entry {
        code: Code::GptRepairFailed,
        id: "E4005",
        exit: EXIT_RUNTIME_ERROR,
        title: "Réparation du GPT impossible",
        causes: &[
            "aucun en-tête GPT valide ne peut servir de modèle",
            "le support est trop petit pour la table",
        ],
        remedy: "Inspecter le support avec `bonk debug layout` et réécrire l'image si besoin.",
    },
    Entry {
        code: Code::LogSettingInvalid,
        id: "E5001",
        exit: EXIT_USAGE,
        title: "Réglage de journal invalide",
        causes: &["valeur de `BONK_LOG` mal formée (composant ou niveau inconnu)"],
        remedy: "Utiliser `composant=niveau` avec un composant parmi cli, gui, hal, io, write, net, render et un niveau parmi off, error, warn, info, debug, trace.",
    },
    Entry {
        code: Code::ConfigInvalid,
        id: "E5002",
        exit: EXIT_USAGE,
        title: "Configuration invalide",
        causes: &["clé inconnue, valeur refusée, ou fichier de configuration illisible"],
        remedy: "Lister les clés avec `bonk config list` ; `bonk config reset` revient aux valeurs par défaut.",
    },
    Entry {
        code: Code::WindowUnavailable,
        id: "E5003",
        exit: EXIT_RUNTIME_ERROR,
        title: "Fenêtre impossible à ouvrir",
        causes: &[
            "pas d'affichage graphique (SSH, serveur)",
            "aucun rendu utilisable, même logiciel",
        ],
        remedy: "Utiliser la ligne de commande ou le service (`bonk serve`), qui n'exigent aucun affichage.",
    },
    Entry {
        code: Code::OutputFailed,
        id: "E5004",
        exit: EXIT_RUNTIME_ERROR,
        title: "Écriture de la sortie impossible",
        causes: &["sortie fermée (tube coupé), disque plein, fichier de sortie déjà existant"],
        remedy: "Vérifier la destination de la sortie ; `--output` n'écrase jamais un fichier existant.",
    },
    Entry {
        code: Code::FixtureInvalid,
        id: "E5005",
        exit: EXIT_USAGE,
        title: "Capture d'énumération invalide",
        causes: &[
            "le fichier donné à `--fixture` n'est pas une capture produite par `diagnose --capture`",
        ],
        remedy: "Produire la capture avec `bonk diagnose --capture` sur la machine à analyser.",
    },
    Entry {
        code: Code::LogUnusable,
        id: "E5006",
        exit: EXIT_RUNTIME_ERROR,
        title: "Fichier de journal inutilisable",
        causes: &["`--log-file` désigne un chemin non inscriptible"],
        remedy: "Choisir un chemin dans un dossier où l'on peut écrire.",
    },
    Entry {
        code: Code::ProtocolError,
        id: "E6001",
        exit: EXIT_RUNTIME_ERROR,
        title: "Requête du service invalide",
        causes: &["ligne qui n'est pas du JSON, méthode inconnue ou paramètres invalides"],
        remedy: "Voir `bonk schema serve` pour le format des requêtes et des réponses.",
    },
];

impl Code {
    pub(crate) fn entry(self) -> &'static Entry {
        // Le catalogue contient tous les codes : vérifié par un test.
        CATALOG
            .iter()
            .find(|entry| entry.code == self)
            .unwrap_or(&CATALOG[0])
    }

    pub(crate) fn id(self) -> &'static str {
        self.entry().id
    }

    pub(crate) fn exit(self) -> u8 {
        self.entry().exit
    }

    /// Le code qui décrit un échec d'énumération.
    pub(crate) fn of_enumeration(error: &EnumerateError) -> Self {
        match error {
            EnumerateError::Unsupported { .. } => Self::EnumerationUnsupported,
            _ => Self::EnumerationFailed,
        }
    }
}

/// Écrit l'erreur sur `err` au format `bonk : erreur : <message> [Exxxx]` et rend le code de sortie.
pub(crate) fn report(err: &mut dyn Write, code: Code, message: impl Display) -> u8 {
    let _ = writeln!(err, "bonk : erreur : {message} [{}]", code.id());
    code.exit()
}

/// Cherche un code, avec ou sans majuscule (`e1003` comme `E1003`).
pub(crate) fn lookup(text: &str) -> Option<&'static Entry> {
    CATALOG
        .iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(text.trim()))
}

/// Explique un code d'erreur
#[derive(Debug, Args)]
// `long_about = None` : sans lui, clap reprendrait le commentaire ci-dessus comme texte long.
#[command(long_about = None)]
pub(crate) struct ExplainArgs {
    /// Code à expliquer (`E1003`) ; sans code, liste tous les codes
    code: Option<String>,

    /// Sortie JSON
    #[arg(long)]
    json: bool,
}

#[derive(Serialize)]
struct EntryJson {
    code: &'static str,
    exit_code: u8,
    title: &'static str,
    causes: &'static [&'static str],
    remedy: &'static str,
}

impl From<&Entry> for EntryJson {
    fn from(entry: &Entry) -> Self {
        Self {
            code: entry.id,
            exit_code: entry.exit,
            title: entry.title,
            causes: entry.causes,
            remedy: entry.remedy,
        }
    }
}

#[derive(Serialize)]
struct ExplainReport {
    schema_version: u32,
    codes: Vec<EntryJson>,
}

impl ExplainArgs {
    pub(crate) fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }
}

/// `bonk explain`. Renvoie `false` si le code demandé n'existe pas.
pub(crate) fn explain(args: &ExplainArgs, out: &mut dyn Write) -> io::Result<bool> {
    let entries: Vec<&Entry> = match &args.code {
        Some(code) => match lookup(code) {
            Some(entry) => vec![entry],
            None => return Ok(false),
        },
        None => CATALOG.iter().collect(),
    };
    if args.json {
        let report = ExplainReport {
            schema_version: 1,
            codes: entries.iter().map(|e| EntryJson::from(*e)).collect(),
        };
        serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
        return writeln!(out).map(|()| true);
    }
    if args.code.is_none() {
        for entry in entries {
            writeln!(
                out,
                "{}  (sortie {})  {}",
                entry.id, entry.exit, entry.title
            )?;
        }
        return Ok(true);
    }
    for entry in entries {
        writeln!(out, "{} : {}", entry.id, entry.title)?;
        writeln!(out, "Code de sortie : {}", entry.exit)?;
        writeln!(out, "Causes probables :")?;
        for cause in entry.causes {
            writeln!(out, "  - {cause}")?;
        }
        writeln!(out, "Remède : {}", entry.remedy)?;
    }
    Ok(true)
}

/// Texte d'un code inconnu, avec la liste des codes existants.
pub(crate) fn unknown_code_message(text: &str) -> String {
    let ids: Vec<&str> = CATALOG.iter().map(|e| e.id).collect();
    format!("code inconnu : « {text} » (codes : {})", ids.join(", "))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn identifiants_uniques_ordonnes_et_bien_formes() {
        let ids: Vec<&str> = CATALOG.iter().map(|e| e.id).collect();
        let unique: BTreeSet<&str> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len(), "identifiant en double : {ids:?}");
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "le catalogue suit l'ordre des numéros");
        for id in ids {
            assert!(
                id.len() == 5 && id.starts_with('E') && id[1..].bytes().all(|b| b.is_ascii_digit()),
                "{id}"
            );
        }
    }

    #[test]
    fn chaque_code_a_une_fiche_complete() {
        for entry in CATALOG {
            assert!(
                !entry.title.is_empty() && !entry.remedy.is_empty(),
                "{}",
                entry.id
            );
            assert!(!entry.causes.is_empty(), "{}", entry.id);
            assert_eq!(entry.code.entry().id, entry.id);
            assert!(
                (1..=6).contains(&entry.exit),
                "{} : sortie {}",
                entry.id,
                entry.exit
            );
        }
    }

    #[test]
    fn codes_de_sortie_par_classe() {
        assert_eq!(Code::TargetRefused.exit(), 3);
        assert_eq!(Code::IdentityChanged.exit(), 3);
        assert_eq!(Code::VerifyMismatch.exit(), 4);
        assert_eq!(Code::Cancelled.exit(), 5);
        assert_eq!(Code::SourceTooLarge.exit(), 6);
        assert_eq!(Code::ConfirmationRequired.exit(), 2);
        assert_eq!(Code::DiskNotFound.exit(), 1);
    }

    #[test]
    fn le_code_termine_le_message() {
        let mut err = Vec::new();
        let exit = report(&mut err, Code::DiskNotFound, "disque introuvable : « x »");
        assert_eq!(exit, EXIT_RUNTIME_ERROR);
        assert_eq!(
            String::from_utf8_lossy(&err),
            "bonk : erreur : disque introuvable : « x » [E1003]\n"
        );
    }

    #[test]
    fn recherche_insensible_a_la_casse() {
        assert_eq!(lookup("e1003").map(|e| e.id), Some("E1003"));
        assert_eq!(lookup(" E4002 ").map(|e| e.id), Some("E4002"));
        assert!(lookup("E9999").is_none());
    }

    #[test]
    fn explain_un_code_et_la_liste() -> io::Result<()> {
        let mut out = Vec::new();
        let args = ExplainArgs {
            code: Some("e2001".to_owned()),
            json: false,
        };
        assert!(explain(&args, &mut out)?);
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(
            text.starts_with("E2001 : Cible refusée par le filtre de sécurité"),
            "{text}"
        );
        assert!(text.contains("Code de sortie : 3"), "{text}");
        assert!(text.contains("bonk debug filter"), "{text}");

        let mut list = Vec::new();
        explain(
            &ExplainArgs {
                code: None,
                json: false,
            },
            &mut list,
        )?;
        let text = String::from_utf8_lossy(&list).into_owned();
        assert_eq!(text.lines().count(), CATALOG.len());
        assert!(text.contains("E4002  (sortie 4)"), "{text}");

        let mut json = Vec::new();
        explain(
            &ExplainArgs {
                code: Some("E4003".to_owned()),
                json: true,
            },
            &mut json,
        )?;
        let value: serde_json::Value = serde_json::from_slice(&json)?;
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["codes"][0]["exit_code"], 5);
        assert!(!explain(
            &ExplainArgs {
                code: Some("E0".to_owned()),
                json: false
            },
            &mut Vec::new()
        )?);
        Ok(())
    }

    #[test]
    fn echec_d_enumeration_vers_le_bon_code() {
        assert_eq!(
            Code::of_enumeration(&EnumerateError::Unsupported { os: "x" }),
            Code::EnumerationUnsupported
        );
        let malformed = EnumerateError::Malformed {
            what: "a".into(),
            detail: "b".into(),
        };
        assert_eq!(Code::of_enumeration(&malformed), Code::EnumerationFailed);
    }
}
