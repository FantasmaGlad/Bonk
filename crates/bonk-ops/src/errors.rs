//! Codes d'erreur stables et codes de sortie.
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
    DownloadFailed,
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
        title: "Invalid argument value",
        causes: &["a number, offset or length outside the accepted limits"],
        remedy: "Read the command help (`bonk <command> --help`) and correct the value.",
    },
    Entry {
        code: Code::EnumerationFailed,
        id: "E1001",
        exit: EXIT_RUNTIME_ERROR,
        title: "Disk detection failed",
        causes: &[
            "a system source is unreadable (mount points, udev database, disk list)",
            "udev database missing (minimal container, system without udev)",
        ],
        remedy: "Run `bonk debug sources` to see which read fails, then `bonk diagnose` for a full report. Bonk offers nothing rather than guess.",
    },
    Entry {
        code: Code::EnumerationUnsupported,
        id: "E1002",
        exit: EXIT_RUNTIME_ERROR,
        title: "Disk detection is not available on this system",
        causes: &["enumeration only exists on Linux in this version"],
        remedy: "Use a Linux host, or replay a Linux capture with `--fixture`.",
    },
    Entry {
        code: Code::DiskNotFound,
        id: "E1003",
        exit: EXIT_RUNTIME_ERROR,
        title: "Disk not found",
        causes: &[
            "the identifier or path matches no detected disk",
            "the disk was unplugged, or its identifier changed",
        ],
        remedy: "List the identifiers with `bonk list --all` and try again.",
    },
    Entry {
        code: Code::RawDiskUnavailable,
        id: "E1004",
        exit: EXIT_RUNTIME_ERROR,
        title: "Raw disk read failed",
        causes: &[
            "the path is neither an image nor a disk",
            "permission denied, or a system without raw access",
        ],
        remedy: "Check the target with `bonk list --all`, accept the system authorization, or copy the start of the disk into an image.",
    },
    Entry {
        code: Code::TargetRefused,
        id: "E2001",
        exit: EXIT_TARGET_REFUSED,
        title: "Target refused by the safety filter",
        causes: &[
            "the disk holds the system, the active ESP, a swap space or a mount outside user paths",
            "the disk is a RAID, LVM or ZFS member, or holds a stacked volume",
            "the disk is internal or virtual and the matching option is not set",
        ],
        remedy: "See the reasons with `bonk debug filter <disk>`. A forbidden disk can never be written; unmount a manual mount or choose another disk.",
    },
    Entry {
        code: Code::IdentityChanged,
        id: "E2002",
        exit: EXIT_TARGET_REFUSED,
        title: "The disk identity changed before writing",
        causes: &[
            "the disk was unplugged and another plugged in at the same location",
            "the serial number or the size no longer match",
        ],
        remedy: "List the disks again with `bonk list`, check the target and run again.",
    },
    Entry {
        code: Code::UnmountFailed,
        id: "E2003",
        exit: EXIT_RUNTIME_ERROR,
        title: "Could not unmount the target partitions",
        causes: &[
            "a program is still using the volume",
            "insufficient rights to unmount",
        ],
        remedy: "Close the programs using the drive, unmount it by hand, then run again.",
    },
    Entry {
        code: Code::ConfirmationRequired,
        id: "E2004",
        exit: EXIT_USAGE,
        title: "Confirmation required",
        causes: &["a write without `--yes` or `--force`: Bonk never asks a question"],
        remedy: "First describe the operation with `--dry-run`, then run again with `--yes` or `--force`.",
    },
    Entry {
        code: Code::InternalConfirmationRequired,
        id: "E2005",
        exit: EXIT_USAGE,
        title: "Confirmation required for an internal disk",
        causes: &["a write to an internal disk without `--force`: `--yes` is not enough"],
        remedy: "Check the target with `bonk info <disk>`: all its data will be lost. Run again with `--force` to confirm.",
    },
    Entry {
        code: Code::SourceUnreadable,
        id: "E3001",
        exit: EXIT_SOURCE,
        title: "Image unreadable",
        causes: &["file missing, insufficient rights, or not a regular file"],
        remedy: "Check the path and the file permissions.",
    },
    Entry {
        code: Code::SourceFormatUnsupported,
        id: "E3002",
        exit: EXIT_SOURCE,
        title: "Image format not supported",
        causes: &["compressed format recognized but not supported (bzip2, zip...)"],
        remedy: "Decompress the image with a system tool, or provide it as .img, .iso, .gz, .xz or .zst.",
    },
    Entry {
        code: Code::SourceTooLarge,
        id: "E3003",
        exit: EXIT_SOURCE,
        title: "The image is larger than the target",
        causes: &["the decompressed image exceeds the size of the target disk or image"],
        remedy: "Choose a larger drive. Nothing is written past the end of the drive.",
    },
    Entry {
        code: Code::DecompressionFailed,
        id: "E3004",
        exit: EXIT_SOURCE,
        title: "Decompression failed",
        causes: &["truncated or corrupt compressed file"],
        remedy: "Download the image again and check its hash before retrying.",
    },
    Entry {
        code: Code::SourceHashMismatch,
        id: "E3005",
        exit: EXIT_SOURCE,
        title: "The image hash does not match",
        causes: &[
            "corrupt or incomplete download",
            "modified image, or a hash from another version",
        ],
        remedy: "Download the image again from the publisher's site and take its official hash again. Nothing was written.",
    },
    Entry {
        code: Code::ImageNotBootable,
        id: "E3006",
        exit: EXIT_SOURCE,
        title: "The image would not boot if written as is",
        causes: &[
            "ISO without disk boot, like Windows installation ISOs",
            "the file is not a disk image",
        ],
        remedy: "Use an image intended for a USB drive. To write these bytes anyway: --mode raw. Nothing was written.",
    },
    Entry {
        code: Code::ImageTruncated,
        id: "E3007",
        exit: EXIT_SOURCE,
        title: "The image is shorter than it declares",
        causes: &["interrupted download"],
        remedy: "Download the image again and check its hash (--sha256). To write these bytes anyway: --mode raw. Nothing was written.",
    },
    Entry {
        code: Code::DownloadFailed,
        id: "E3008",
        exit: EXIT_SOURCE,
        title: "The image download failed",
        causes: &[
            "network down or server unavailable",
            "wrong address, or not https://",
            "cache directory full or not writable",
        ],
        remedy: "Run the same command again: the download resumes where it stopped. Nothing was written to the drive.",
    },
    Entry {
        code: Code::WriteFailed,
        id: "E4001",
        exit: EXIT_RUNTIME_ERROR,
        title: "Write failed",
        causes: &["drive I/O error (faulty drive, unplugged during the write)"],
        remedy: "The drive is in an undetermined state: write it again entirely, on another drive if the error returns.",
    },
    Entry {
        code: Code::VerifyMismatch,
        id: "E4002",
        exit: EXIT_VERIFY_FAILED,
        title: "Verification after writing failed",
        causes: &["the bytes read back do not match the image: faulty drive or partial write"],
        remedy: "Write the image again; if the error persists, replace the drive.",
    },
    Entry {
        code: Code::Cancelled,
        id: "E4003",
        exit: EXIT_CANCELLED,
        title: "Operation cancelled",
        causes: &["cancellation requested (Ctrl+C or a cancel request)"],
        remedy: "The drive is partially written and not bootable: run the full write again.",
    },
    Entry {
        code: Code::DeviceOpenDenied,
        id: "E4004",
        exit: EXIT_RUNTIME_ERROR,
        title: "Opening the disk was refused",
        causes: &[
            "system authorization refused or cancelled",
            "no polkit agent in the session, or UDisks2 service missing",
        ],
        remedy: "Accept the system authorization when it appears. Without a polkit agent (SSH) or without UDisks2: run the command again with `sudo`.",
    },
    Entry {
        code: Code::GptRepairFailed,
        id: "E4005",
        exit: EXIT_RUNTIME_ERROR,
        title: "GPT repair failed",
        causes: &[
            "no valid GPT header can serve as a model",
            "the drive is too small for the table",
        ],
        remedy: "Inspect the drive with `bonk debug layout` and write the image again if needed.",
    },
    Entry {
        code: Code::LogSettingInvalid,
        id: "E5001",
        exit: EXIT_USAGE,
        title: "Invalid log setting",
        causes: &["malformed `BONK_LOG` value (unknown component or level)"],
        remedy: "Use `component=level` with a component among cli, gui, hal, io, write, net, render and a level among off, error, warn, info, debug, trace.",
    },
    Entry {
        code: Code::ConfigInvalid,
        id: "E5002",
        exit: EXIT_USAGE,
        title: "Invalid configuration",
        causes: &["unknown key, refused value, or unreadable configuration file"],
        remedy: "List the keys with `bonk config list`; `bonk config reset` restores the defaults.",
    },
    Entry {
        code: Code::WindowUnavailable,
        id: "E5003",
        exit: EXIT_RUNTIME_ERROR,
        title: "The window could not be opened",
        causes: &[
            "no graphical display (SSH, server)",
            "no usable renderer, not even software",
        ],
        remedy: "Use the command line or the service (`bonk serve`), which need no display.",
    },
    Entry {
        code: Code::OutputFailed,
        id: "E5004",
        exit: EXIT_RUNTIME_ERROR,
        title: "Could not write the output",
        causes: &["output closed (broken pipe), disk full, output file already exists"],
        remedy: "Check the output destination; `--output` never overwrites an existing file.",
    },
    Entry {
        code: Code::FixtureInvalid,
        id: "E5005",
        exit: EXIT_USAGE,
        title: "Invalid enumeration capture",
        causes: &[
            "the file given to `--fixture` is not a capture produced by `diagnose --capture`",
        ],
        remedy: "Produce the capture with `bonk diagnose --capture` on the machine to analyze.",
    },
    Entry {
        code: Code::LogUnusable,
        id: "E5006",
        exit: EXIT_RUNTIME_ERROR,
        title: "Log file unusable",
        causes: &["`--log-file` names a path that cannot be written"],
        remedy: "Choose a path in a writable directory.",
    },
    Entry {
        code: Code::ProtocolError,
        id: "E6001",
        exit: EXIT_RUNTIME_ERROR,
        title: "Invalid service request",
        causes: &["a line that is not JSON, an unknown method or invalid parameters"],
        remedy: "See `bonk schema serve` for the request and response format.",
    },
    Entry {
        code: Code::FormatFailed,
        id: "E7001",
        exit: EXIT_RUNTIME_ERROR,
        title: "Format failed",
        causes: &[
            "drive too small or too large for the requested table or filesystem",
            "write error, or read-back different from what was written",
            "system tool failure",
        ],
        remedy: "See `bonk format --dry-run`: it says what would be done, writing nothing. Another `--fs` or `--table` may fit.",
    },
    Entry {
        code: Code::FilesystemUnavailable,
        id: "E7002",
        exit: EXIT_RUNTIME_ERROR,
        title: "Filesystem unavailable on this host",
        causes: &[
            "exFAT, NTFS and ext4 go through system tools: the tool or service is missing",
            "`--simulate`: only FAT32 can be written to an image file",
        ],
        remedy: "See `bonk capabilities`. Install the named tool, or choose `--fs fat32`, written by Bonk itself.",
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
