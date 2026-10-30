//! Codes d'erreur stables et codes de sortie (CDC §9.1, D-28).
//!
//! Chaque erreur porte un code `Exxxx` : un outil le retrouve sans interpréter le texte, et le catalogue
//! en donne le sens, les causes probables et le remède. Les codes ne sont jamais réutilisés ni renumérotés.

use bonk_hal::EnumerateError;

/// Code de sortie d'une erreur d'exécution générale.
pub const EXIT_RUNTIME_ERROR: u8 = 1;
/// Code de sortie d'une erreur d'usage, celui de clap.
pub const EXIT_USAGE: u8 = 2;
/// Cible refusée par le filtre de sécurité, ou dont l'identité a changé.
pub const EXIT_TARGET_REFUSED: u8 = 3;
/// La vérification après écriture a échoué.
pub const EXIT_VERIFY_FAILED: u8 = 4;
/// Opération annulée par l'utilisateur.
pub const EXIT_CANCELLED: u8 = 5;
/// Source (image) illisible, de format inconnu ou trop grande.
pub const EXIT_SOURCE: u8 = 6;

/// Une erreur connue de Bonk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    InvalidArgument,
    EnumerationFailed,
    EnumerationUnsupported,
    DiskNotFound,
    RawDiskUnavailable,
    TargetRefused,
    IdentityChanged,
    UnmountFailed,
    ConfirmationRequired,
    InternalConfirmationRequired,
    SourceUnreadable,
    SourceFormatUnsupported,
    SourceTooLarge,
    DecompressionFailed,
    SourceHashMismatch,
    ImageNotBootable,
    ImageTruncated,
    WriteFailed,
    VerifyMismatch,
    Cancelled,
    DeviceOpenDenied,
    GptRepairFailed,
    FormatFailed,
    FilesystemUnavailable,
    LogSettingInvalid,
    ConfigInvalid,
    WindowUnavailable,
    OutputFailed,
    FixtureInvalid,
    LogUnusable,
    ProtocolError,
}

/// Fiche d'un code, telle que la montre `bonk explain`.
pub struct Entry {
    pub code: Code,
    pub id: &'static str,
    pub exit: u8,
    pub title: &'static str,
    pub causes: &'static [&'static str],
    pub remedy: &'static str,
}

/// Le catalogue. Ordre = ordre des numéros.
pub const CATALOG: &[Entry] = &[
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
        title: "Lecture brute du disque impossible",
        causes: &[
            "chemin qui ne désigne ni une image ni un disque",
            "droits refusés, ou système sans accès brut",
        ],
        remedy: "Vérifier la cible avec `bonk list --all`, accepter l'autorisation du système, ou copier le début du disque dans une image.",
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
        causes: &["une écriture sans `--yes` ni `--force` : Bonk ne pose jamais de question"],
        remedy: "Décrire d'abord l'opération avec `--dry-run`, puis relancer avec `--yes` ou `--force`.",
    },
    Entry {
        code: Code::InternalConfirmationRequired,
        id: "E2005",
        exit: EXIT_USAGE,
        title: "Confirmation requise pour un disque interne",
        causes: &["écriture sur un disque interne sans `--force` : `--yes` ne suffit pas"],
        remedy: "Vérifier la cible avec `bonk info <disque>` : toutes ses données seront perdues. Relancer avec `--force` pour le confirmer.",
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
        code: Code::SourceHashMismatch,
        id: "E3005",
        exit: EXIT_SOURCE,
        title: "L'empreinte de l'image ne correspond pas",
        causes: &[
            "téléchargement corrompu ou incomplet",
            "image modifiée, ou empreinte d'une autre version",
        ],
        remedy: "Retélécharger l'image depuis le site de l'éditeur et reprendre son empreinte officielle. Rien n'a été écrit.",
    },
    Entry {
        code: Code::ImageNotBootable,
        id: "E3006",
        exit: EXIT_SOURCE,
        title: "L'image ne démarrerait pas écrite telle quelle",
        causes: &[
            "ISO sans amorçage de disque, comme les ISO d'installation de Windows",
            "fichier qui n'est pas une image de disque",
        ],
        remedy: "Utiliser une image prévue pour une clé USB. Pour écrire quand même ces octets : --mode raw. Rien n'a été écrit.",
    },
    Entry {
        code: Code::ImageTruncated,
        id: "E3007",
        exit: EXIT_SOURCE,
        title: "L'image est plus courte que ce qu'elle annonce",
        causes: &["téléchargement interrompu"],
        remedy: "Retélécharger l'image et contrôler son empreinte (--sha256). Pour écrire quand même ces octets : --mode raw. Rien n'a été écrit.",
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
        code: Code::FormatFailed,
        id: "E4006",
        exit: EXIT_RUNTIME_ERROR,
        title: "Formatage impossible",
        causes: &[
            "support trop petit ou trop grand pour la table ou le système de fichiers demandés",
            "erreur d'écriture, ou relecture différente de ce qui a été écrit",
            "échec de l'outil du système",
        ],
        remedy: "Voir `bonk format --dry-run` : il dit ce qui serait fait, sans rien écrire. Un autre `--fs` ou `--table` peut convenir.",
    },
    Entry {
        code: Code::FilesystemUnavailable,
        id: "E4007",
        exit: EXIT_RUNTIME_ERROR,
        title: "Système de fichiers indisponible sur cet hôte",
        causes: &[
            "exFAT, NTFS et ext4 passent par les outils du système : l'outil ou le service manque",
            "`--simulate` : seul FAT32 s'écrit sur un fichier image",
        ],
        remedy: "Voir `bonk capabilities`. Installer l'outil nommé, ou choisir `--fs fat32`, écrit par Bonk lui-même.",
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
    pub fn entry(self) -> &'static Entry {
        // Le catalogue contient tous les codes : vérifié par un test.
        CATALOG
            .iter()
            .find(|entry| entry.code == self)
            .unwrap_or(&CATALOG[0])
    }

    pub fn id(self) -> &'static str {
        self.entry().id
    }

    pub fn exit(self) -> u8 {
        self.entry().exit
    }

    /// Le code qui décrit un échec d'énumération.
    pub fn of_enumeration(error: &EnumerateError) -> Self {
        match error {
            EnumerateError::Unsupported { .. } => Self::EnumerationUnsupported,
            _ => Self::EnumerationFailed,
        }
    }
}

/// Cherche un code, avec ou sans majuscule (`e1003` comme `E1003`).
pub fn lookup(text: &str) -> Option<&'static Entry> {
    CATALOG
        .iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(text.trim()))
}
