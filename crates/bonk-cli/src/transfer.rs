//! Commandes `bonk write`, `bonk verify` et `bonk repair-gpt` : arguments, progression, affichage.
//!
//! La logique est dans `bonk_ops::job` ; ce module la présente. Avec `--progress json`, la sortie
//! standard est un flux de lignes JSON (événements `progress` puis un événement `result`), les mêmes
//! événements que la barre de la fenêtre ; sinon le résultat est du texte ou, avec `--json`, un JSON.

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bonk_core::analysis::{ImageKind, ImageReport};
use bonk_core::write::{Control, Phase, Progress, VerifyReport};
use bonk_hal::{Elevation, FilterOptions, format_size};
use clap::{Args, ValueEnum};
use schemars::JsonSchema;
use serde::Serialize;

use crate::config::ViewDefaults;
use crate::errors::{Code, report};
use bonk_ops::job::{
    self, Deps, JobError, RepairDone, RepairJob, Target, TraceIo, VerifyDone, VerifyJob, WriteDone,
    WriteJob, WriteMode, WriteOutcome,
};

/// Niveau de `--trace-io` (voir [`TraceIo`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum TraceIoArg {
    /// Décalage, longueur, durée et somme de contrôle de chaque accès
    Metadata,
    /// Avec les 16 premiers octets : réservé aux fichiers image
    Data,
}

impl From<TraceIoArg> for TraceIo {
    fn from(arg: TraceIoArg) -> Self {
        match arg {
            TraceIoArg::Metadata => Self::Metadata,
            TraceIoArg::Data => Self::Data,
        }
    }
}

/// Valeur de `--elevate` (voir [`Elevation`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ElevateArg {
    /// Demande l'autorisation du système au moment d'ouvrir le disque
    Auto,
    /// Droits de l'appelant seulement
    Never,
}

impl From<ElevateArg> for Elevation {
    fn from(arg: ElevateArg) -> Self {
        match arg {
            ElevateArg::Auto => Self::Auto,
            ElevateArg::Never => Self::Never,
        }
    }
}

/// Mode de `--mode` (voir [`WriteMode`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ModeArg {
    /// Refuse une image tronquée ou qui ne démarrerait pas écrite telle quelle
    Auto,
    /// Écrit les octets tels quels, quelle que soit l'analyse
    Raw,
}

impl From<ModeArg> for WriteMode {
    fn from(arg: ModeArg) -> Self {
        match arg {
            ModeArg::Auto => Self::Auto,
            ModeArg::Raw => Self::Raw,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ProgressMode {
    /// Une ligne de progression sur la sortie d'erreur si c'est un terminal, rien sinon
    Auto,
    /// Un événement JSON par ligne sur la sortie standard
    Json,
    /// Aucune progression
    None,
}

/// Options communes aux opérations sur un disque.
#[derive(Debug, Args)]
pub(crate) struct CommonArgs {
    /// Identifiant stable du disque (voir `bonk list`) ; avec --simulate, chemin du fichier image cible
    pub(crate) target: String,

    /// La cible est un fichier image (créé s'il n'existe pas, avec --size) et non un disque : rejoue
    /// l'opération sans matériel
    #[arg(long)]
    pub(crate) simulate: bool,

    /// Taille de l'image cible créée par --simulate (décimal ou 0x...) ; par défaut celle de l'image
    /// source quand elle est brute
    #[arg(long, value_name = "OCTETS", value_parser = crate::debug::parse_number)]
    size: Option<u64>,

    /// Autorise les disques internes non système (avertissement renforcé)
    #[arg(long)]
    show_internal: bool,

    /// Mode test : autorise les disques virtuels
    #[arg(long)]
    allow_virtual: bool,

    /// Journalise chaque accès bloc (décalage, durée, somme de contrôle) ; `=data` ajoute les
    /// premiers octets, pour un fichier image seulement
    #[arg(long, value_enum, num_args = 0..=1, default_missing_value = "metadata", require_equals = true)]
    pub(crate) trace_io: Option<TraceIoArg>,

    /// Droits manquants : auto (demande au système) ou never (droits de l'appelant seulement)
    #[arg(long, value_enum, default_value_t = ElevateArg::Auto)]
    pub(crate) elevate: ElevateArg,

    /// Progression : auto, json (un événement par ligne) ou none
    #[arg(long, value_enum, default_value_t = ProgressMode::Auto)]
    progress: ProgressMode,

    /// Résultat en JSON
    #[arg(long)]
    json: bool,
}

impl CommonArgs {
    pub(crate) fn target(&self) -> Target {
        if self.simulate {
            Target::Image {
                path: PathBuf::from(&self.target),
                size: self.size,
            }
        } else {
            Target::Disk(self.target.clone())
        }
    }

    fn options(&self, view: &ViewDefaults) -> FilterOptions {
        FilterOptions {
            show_internal: self.show_internal || view.show_internal,
            allow_virtual: self.allow_virtual || view.allow_virtual,
        }
    }

    /// Pour écrire, seules les options de la commande comptent : un réglage ou une variable oubliés règlent
    /// l'affichage, ils n'autorisent jamais à effacer un disque de plus.
    pub(crate) fn write_options(&self) -> FilterOptions {
        FilterOptions {
            show_internal: self.show_internal,
            allow_virtual: self.allow_virtual,
        }
    }
}

/// Écrit une image sur un disque (ou un fichier image avec --simulate), puis la vérifie
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct WriteArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Image à écrire : .img, .iso, .gz, .xz ou .zst (le format se reconnaît à son contenu)
    source: PathBuf,

    /// Confirme l'écriture : sans --yes ni --force, Bonk ne fait rien (il ne pose jamais de question)
    #[arg(long)]
    yes: bool,

    /// Décrit ce qui serait fait sans rien écrire
    #[arg(long)]
    dry_run: bool,

    /// Empreinte SHA-256 publiée pour ce fichier : contrôlée avant tout geste sur le support
    #[arg(long, value_name = "HEX")]
    sha256: Option<String>,

    /// auto : analyse l'image et refuse ce qui ne démarrerait pas ; raw : écrit les octets tels quels
    #[arg(long, value_enum, default_value_t = ModeArg::Auto)]
    mode: ModeArg,

    /// Ne relit pas le support après l'écriture
    #[arg(long)]
    no_verify: bool,

    /// Ne répare pas le GPT de secours après l'écriture
    #[arg(long)]
    no_repair_gpt: bool,

    /// Confirme comme --yes, et autorise un disque interne admis par --show-internal, sans question.
    /// Jamais un disque système : le filtre de sécurité reste souverain
    #[arg(long)]
    force: bool,
}

impl WriteArgs {
    /// Cette commande ouvrirait-elle un vrai disque ? (`--dry-run` et `--simulate` : non)
    pub(crate) fn touches_real_disk(&self) -> bool {
        !self.common.simulate && !self.dry_run
    }
}

/// Compare un disque (ou une image) à une image, sans rien écrire
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct VerifyArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Image de référence
    source: PathBuf,
}

impl VerifyArgs {
    pub(crate) fn touches_real_disk(&self) -> bool {
        !self.common.simulate
    }
}

/// Répare le GPT d'un disque (ou d'une image) : secours à la fin du support, principal restauré
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct RepairArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Confirme la réparation
    #[arg(long)]
    yes: bool,

    /// Dit s'il y a quelque chose à réparer, sans rien écrire
    #[arg(long)]
    dry_run: bool,

    /// Confirme comme --yes, et autorise un disque interne admis par --show-internal, sans question.
    /// Jamais un disque système : le filtre de sécurité reste souverain
    #[arg(long)]
    force: bool,
}

impl RepairArgs {
    pub(crate) fn touches_real_disk(&self) -> bool {
        !self.common.simulate
    }
}

/// Événement de progression, tel qu'il sort en `--progress json`.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ProgressEvent<'a> {
    schema_version: u32,
    event: &'static str,
    #[serde(flatten)]
    progress: &'a Progress,
}

pub(crate) fn schema_write() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(WriteDone)).unwrap_or_default()
}

pub(crate) fn schema_plan() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(job::Plan)).unwrap_or_default()
}

pub(crate) fn schema_verify() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(VerifyDone)).unwrap_or_default()
}

pub(crate) fn schema_repair() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(RepairDone)).unwrap_or_default()
}

pub(crate) fn schema_progress() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ProgressEvent)).unwrap_or_default()
}

/// Rappel de progression selon le mode choisi.
fn sink<'a>(
    mode: ProgressMode,
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
) -> impl FnMut(&Progress) -> Control + 'a {
    let tty = io::stderr().is_terminal();
    let mut last = Instant::now() - Duration::from_secs(1);
    move |p| {
        let boundary = matches!(p.phase, Phase::Flush) || p.bytes_done == 0;
        match mode {
            ProgressMode::None => {}
            ProgressMode::Json => {
                // Chaque événement part tel quel : un outil qui lit le flux veut tous les paliers.
                let event = ProgressEvent {
                    schema_version: job::SCHEMA_VERSION,
                    event: "progress",
                    progress: p,
                };
                if let Ok(line) = serde_json::to_string(&event) {
                    let _ = writeln!(out, "{line}");
                    let _ = out.flush();
                }
            }
            ProgressMode::Auto if tty => {
                if boundary || last.elapsed() >= Duration::from_millis(250) {
                    last = Instant::now();
                    let step = match p.phase {
                        Phase::Authorize => "autorisation du système",
                        Phase::Source => "contrôle de l'empreinte",
                        Phase::Write => "écriture",
                        Phase::Flush => "vidage des tampons",
                        Phase::Verify => "vérification",
                    };
                    let total = p
                        .bytes_total
                        .map_or_else(String::new, |t| format!(" / {}", format_size(t)));
                    let _ = write!(
                        err,
                        "\r{step} : {}{total} ({}/s)   ",
                        format_size(p.bytes_done),
                        format_size(p.bytes_per_second as u64)
                    );
                    let _ = err.flush();
                }
            }
            ProgressMode::Auto => {}
        }
        Control::Continue
    }
}

fn end_progress_line(mode: ProgressMode, err: &mut dyn Write) {
    if mode == ProgressMode::Auto && io::stderr().is_terminal() {
        let _ = writeln!(err);
    }
}

fn print_json(out: &mut dyn Write, value: &impl Serialize, compact: bool) -> io::Result<()> {
    if compact {
        writeln!(
            out,
            "{}",
            serde_json::to_string(value).map_err(io::Error::from)?
        )
    } else {
        serde_json::to_writer_pretty(&mut *out, value).map_err(io::Error::from)?;
        writeln!(out)
    }
}

pub(crate) fn target_line(info: &job::TargetInfo) -> String {
    match &info.id {
        Some(id) => format!("disque {id} ({})", info.path),
        None => format!("image {}", info.path),
    }
}

fn image_line(report: &ImageReport) -> String {
    let kind = match report.kind {
        ImageKind::HybridIso => "ISO hybride",
        ImageKind::BootableDisk => "image de disque amorçable",
        ImageKind::DataDisk => "image sans amorçage de PC (carte ARM ou données)",
        ImageKind::PlainIso => "ISO sans amorçage de disque",
        ImageKind::Unknown => "non reconnue",
    };
    let boot: Vec<&str> = [(report.bios, "BIOS"), (report.uefi, "UEFI")]
        .into_iter()
        .filter_map(|(present, name)| present.then_some(name))
        .collect();
    if boot.is_empty() {
        kind.to_owned()
    } else {
        format!("{kind} ({})", boot.join(", "))
    }
}

fn seconds(ms: u64) -> String {
    format!("{},{:01} s", ms / 1000, ms % 1000 / 100)
}

pub(crate) fn finish<T: Serialize>(
    result: Result<T, JobError>,
    args: &CommonArgs,
    out: &mut dyn Write,
    err: &mut dyn Write,
    text: impl FnOnce(&T, &mut dyn Write) -> io::Result<()>,
) -> u8 {
    end_progress_line(args.progress, err);
    match result {
        Err(e) => report(err, e.code, e.message),
        Ok(value) => {
            let written = if args.progress == ProgressMode::Json {
                print_json(out, &value, true)
            } else if args.json {
                print_json(out, &value, false)
            } else {
                text(&value, out)
            };
            match written {
                Ok(()) => 0,
                Err(e) => report(err, Code::OutputFailed, e),
            }
        }
    }
}

pub(crate) fn run_write(
    args: &WriteArgs,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    bonk_hal::interrupt::install();
    let job = WriteJob {
        target: args.common.target(),
        source: args.source.clone(),
        options: args.common.write_options(),
        verify: !args.no_verify,
        repair_gpt: !args.no_repair_gpt,
        trace_io: args.common.trace_io.map(TraceIo::from),
        dry_run: args.dry_run,
        yes: args.yes,
        expected_sha256: args.sha256.clone(),
        mode: args.mode.into(),
        elevation: args.common.elevate.into(),
        force: args.force,
    };
    log::debug!(target: "bonk::cli", target_kind = if args.common.simulate { "image" } else { "disk" }, dry_run = args.dry_run; "command write");
    let result = {
        let mut progress = sink(args.common.progress, &mut *out, &mut *err);
        job::write(&job, deps, &mut progress)
    };
    finish(
        result,
        &args.common,
        out,
        err,
        |outcome, out| match outcome {
            WriteOutcome::Plan(plan) => {
                writeln!(out, "Plan (rien n'est écrit) :")?;
                writeln!(out, "  Cible  : {}", target_line(&plan.target))?;
                writeln!(
                    out,
                    "  Source : {} ({}{})",
                    plan.source.path,
                    plan.source.format,
                    plan.source
                        .size_bytes
                        .map_or_else(String::new, |s| format!(", {}", format_size(s)))
                )?;
                writeln!(out, "  Image  : {}", image_line(&plan.image))?;
                if plan.sha256_checked {
                    writeln!(out, "  Empreinte du fichier : conforme")?;
                }
                if !plan.unmount.is_empty() {
                    writeln!(out, "  Démontage : {}", plan.unmount.join(", "))?;
                }
                writeln!(out, "  Étapes : {}", plan.steps.join(", "))
            }
            WriteOutcome::Done(done) => {
                writeln!(
                    out,
                    "Écriture terminée : {} en {} ({}/s)",
                    format_size(done.write.bytes),
                    seconds(done.write.elapsed_ms),
                    format_size(done.write.bytes_per_second as u64)
                )?;
                writeln!(out, "  Cible  : {}", target_line(&done.target))?;
                writeln!(
                    out,
                    "  Source : {} ({})",
                    done.source.path, done.source.format
                )?;
                writeln!(out, "  Image  : {}", image_line(&done.image))?;
                if done.sha256_checked {
                    writeln!(out, "  Empreinte du fichier : conforme")?;
                }
                if !done.unmounted.is_empty() {
                    writeln!(out, "  Démonté : {}", done.unmounted.join(", "))?;
                }
                writeln!(out, "  SHA-256 : {}", done.write.sha256)?;
                match &done.verify {
                    Some(v) => {
                        writeln!(out, "  Vérification : réussie ({} octets relus)", v.bytes)?;
                        cache_warning(v, out)?;
                    }
                    None => writeln!(out, "  Vérification : non demandée")?,
                }
                match &done.gpt_repair {
                    Some(r) => writeln!(out, "  GPT : réparé ({})", r.actions.join(", ")),
                    None if args.no_repair_gpt => writeln!(out, "  GPT : contrôle non demandé"),
                    None => writeln!(out, "  GPT : rien à réparer"),
                }
            }
        },
    )
}

pub(crate) fn run_verify(
    args: &VerifyArgs,
    view: &ViewDefaults,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    bonk_hal::interrupt::install();
    let job = VerifyJob {
        target: args.common.target(),
        source: args.source.clone(),
        options: args.common.options(view),
        trace_io: args.common.trace_io.map(TraceIo::from),
        elevation: args.common.elevate.into(),
    };
    let result = {
        let mut progress = sink(args.common.progress, &mut *out, &mut *err);
        job::verify(&job, deps, &mut progress)
    };
    finish(result, &args.common, out, err, |done, out| {
        writeln!(
            out,
            "Vérification réussie : {} identiques à l'image ({})",
            format_size(done.verify.bytes),
            target_line(&done.target)
        )?;
        if !done.ignored_ranges.is_empty() {
            let octets: u64 = done.ignored_ranges.iter().map(|r| r.length).sum();
            writeln!(
                out,
                "  Exclus de la comparaison : {octets} octets de champs du GPT que sa réparation réécrit"
            )?;
        }
        cache_warning(&done.verify, out)?;
        writeln!(out, "  SHA-256 : {}", done.verify.sha256)
    })
}

fn cache_warning(report: &VerifyReport, out: &mut dyn Write) -> io::Result<()> {
    if report.cache_dropped {
        return Ok(());
    }
    writeln!(
        out,
        "  Attention : le cache du système n'a pas pu être vidé, la relecture a pu lire la mémoire et non le support"
    )
}

pub(crate) fn run_repair(
    args: &RepairArgs,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    bonk_hal::interrupt::install();
    let job = RepairJob {
        target: args.common.target(),
        options: args.common.write_options(),
        trace_io: args.common.trace_io.map(TraceIo::from),
        dry_run: args.dry_run,
        yes: args.yes,
        elevation: args.common.elevate.into(),
        force: args.force,
    };
    let result = job::repair(&job, deps);
    finish(result, &args.common, out, err, |done, out| {
        writeln!(
            out,
            "{} : {}",
            target_line(&done.target),
            if done.needs_repair {
                "GPT à réparer"
            } else {
                "GPT en ordre"
            }
        )?;
        if !done.findings_before.is_empty() {
            writeln!(
                out,
                "  Constats avant : {}",
                done.findings_before.join(", ")
            )?;
        }
        if let Some(r) = &done.repair {
            writeln!(out, "  Réparé : {}", r.actions.join(", "))?;
        }
        if let Some(after) = &done.findings_after {
            writeln!(
                out,
                "  Constats après : {}",
                if after.is_empty() {
                    "aucun".to_owned()
                } else {
                    after.join(", ")
                }
            )?;
        }
        Ok(())
    })
}
