//! Journal de Bonk : options globales, filtre par composant, formats texte et JSON.
//!
//! Tout le code émet par la façade `log` : nos crates, mais aussi Iced, wgpu et winit, ce qui donne le
//! moteur de rendu retenu et les erreurs de pilote dans le même flux. Seul ce module sait où va le
//! journal. Il est local : rien n'est jamais envoyé.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, ValueEnum};
use log::kv::{Key, Value, VisitSource};
use log::{LevelFilter, Log, Metadata, Record};

/// Composants dont on règle le niveau séparément.
const COMPONENTS: [&str; 7] = ["cli", "gui", "hal", "io", "write", "net", "render"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum LogFormat {
    Text,
    Json,
}

/// Options de journal communes à toutes les commandes, et à la fenêtre.
#[derive(Debug, Args)]
pub(crate) struct LogArgs {
    /// Detailed log (-v), then very detailed (-vv) on standard error
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Keep only the result: no progress line, and only errors in the log
    #[arg(short = 'q', long, global = true, conflicts_with = "verbose")]
    quiet: bool,

    /// Also write the log to this file (appended)
    #[arg(long, value_name = "PATH", global = true)]
    log_file: Option<PathBuf>,

    /// Log format: text, or one JSON event per line
    #[arg(long, value_enum, global = true)]
    log_format: Option<LogFormat>,
}

impl LogArgs {
    pub(crate) fn quiet(&self) -> bool {
        self.quiet
    }
}

/// Niveaux par composant. Sans réglage explicite : `-v` donne `debug` à Bonk et `info` au rendu,
/// `-vv` donne `trace` et `debug` ; sans `-v`, avertissements de Bonk et erreurs du rendu seulement
/// (les avertissements des pilotes graphiques sont du bruit pour l'utilisateur).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Filter {
    default: LevelFilter,
    render: LevelFilter,
    per_component: BTreeMap<String, LevelFilter>,
}

impl Filter {
    /// `verbose` vient de `-v`, `spec` de `BONK_LOG` : `gui=debug,hal=trace`, ou un niveau seul
    /// (`debug`) qui vaut pour tous les composants.
    pub(crate) fn new(verbose: u8, spec: Option<&str>) -> Result<Self, String> {
        let (default, render) = match verbose {
            0 => (LevelFilter::Warn, LevelFilter::Error),
            1 => (LevelFilter::Debug, LevelFilter::Info),
            _ => (LevelFilter::Trace, LevelFilter::Debug),
        };
        Self::with_levels(default, render, spec)
    }

    /// `--quiet` : erreurs seulement. `BONK_LOG`, demandé explicitement, garde le dernier mot.
    fn quiet(spec: Option<&str>) -> Result<Self, String> {
        Self::with_levels(LevelFilter::Error, LevelFilter::Error, spec)
    }

    fn with_levels(
        default: LevelFilter,
        render: LevelFilter,
        spec: Option<&str>,
    ) -> Result<Self, String> {
        let mut filter = Self {
            default,
            render,
            per_component: BTreeMap::new(),
        };
        for part in spec.unwrap_or_default().split(',').map(str::trim) {
            if part.is_empty() {
                continue;
            }
            match part.split_once('=') {
                Some((component, level)) => {
                    if !COMPONENTS.contains(&component) {
                        return Err(format!(
                            "unknown log component: '{component}' (expected: {})",
                            COMPONENTS.join(", ")
                        ));
                    }
                    filter
                        .per_component
                        .insert(component.to_owned(), parse_level(level)?);
                }
                None => {
                    let level = parse_level(part)?;
                    filter.default = level;
                    filter.render = level;
                }
            }
        }
        Ok(filter)
    }

    fn level_of(&self, component: &str) -> LevelFilter {
        self.per_component
            .get(component)
            .copied()
            .unwrap_or(if component == "render" {
                self.render
            } else {
                self.default
            })
    }

    /// Niveau maximal demandé, pour `log::set_max_level` : au-delà, rien n'est même construit.
    fn max_level(&self) -> LevelFilter {
        self.per_component
            .values()
            .copied()
            .chain([self.default, self.render])
            .max()
            .unwrap_or(LevelFilter::Off)
    }
}

fn parse_level(text: &str) -> Result<LevelFilter, String> {
    text.parse().map_err(|_| {
        format!("unknown log level: '{text}' (expected: off, error, warn, info, debug, trace)")
    })
}

/// Composant d'un événement, d'après sa cible : `bonk::<composant>` explicite, sinon le crate qui
/// l'émet ; tout ce qui n'est pas à nous (Iced, wgpu, winit...) est du rendu.
fn component_of(target: &str) -> &'static str {
    if let Some(explicit) = target.strip_prefix("bonk::") {
        let name = explicit.split("::").next().unwrap_or_default();
        if let Some(found) = COMPONENTS.iter().find(|c| **c == name) {
            return found;
        }
    }
    match target.split("::").next().unwrap_or_default() {
        "bonk_cli" | "bonk" => "cli",
        "bonk_gui" => "gui",
        "bonk_hal" => "hal",
        "bonk_core" => "io",
        "bonk_customizer" => "write",
        "bonk_catalog" => "net",
        _ => "render",
    }
}

/// Date UTC au format ISO 8601, à la milliseconde (jours civils de Howard Hinnant : pas de dépendance).
pub(crate) fn iso8601(time: SystemTime) -> String {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let millis = since.subsec_millis();
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// Champs structurés d'un événement (`id = "usb-cle"; "message"`).
#[derive(Default)]
struct Fields(Vec<(String, serde_json::Value)>);

impl<'kvs> VisitSource<'kvs> for Fields {
    fn visit_pair(&mut self, key: Key<'kvs>, value: Value<'kvs>) -> Result<(), log::kv::Error> {
        let json = if let Some(n) = value.to_i64() {
            serde_json::Value::from(n)
        } else if let Some(n) = value.to_u64() {
            serde_json::Value::from(n)
        } else if let Some(b) = value.to_bool() {
            serde_json::Value::from(b)
        } else {
            serde_json::Value::from(value.to_string())
        };
        self.0.push((key.as_str().to_owned(), json));
        Ok(())
    }
}

fn fields_of(record: &Record<'_>) -> Fields {
    let mut fields = Fields::default();
    // Une erreur de lecture des champs ne doit pas faire perdre l'événement lui-même.
    let _ = record.key_values().visit(&mut fields);
    fields
}

/// Une ligne de journal, sans retour à la ligne final.
pub(crate) fn format_event(
    format: LogFormat,
    time: SystemTime,
    session: &str,
    record: &Record<'_>,
) -> String {
    let component = component_of(record.target());
    let fields = fields_of(record);
    match format {
        LogFormat::Text => {
            let mut line = format!(
                "{} {:<5} {:<6} {}",
                iso8601(time),
                record.level(),
                component,
                record.args()
            );
            for (key, value) in &fields.0 {
                let _ = match value {
                    serde_json::Value::String(text) => write!(line, " {key}={text:?}"),
                    other => write!(line, " {key}={other}"),
                };
            }
            line
        }
        LogFormat::Json => {
            let mut event = serde_json::Map::new();
            event.insert("ts".into(), iso8601(time).into());
            event.insert(
                "level".into(),
                record.level().as_str().to_lowercase().into(),
            );
            event.insert("component".into(), component.into());
            event.insert("target".into(), record.target().into());
            event.insert("session".into(), session.into());
            event.insert("message".into(), record.args().to_string().into());
            if !fields.0.is_empty() {
                event.insert(
                    "fields".into(),
                    serde_json::Value::Object(fields.0.into_iter().collect()),
                );
            }
            serde_json::Value::Object(event).to_string()
        }
    }
}

/// Puits du journal : filtre par composant, sortie d'erreur et fichier facultatif.
struct Sink {
    filter: Filter,
    format: LogFormat,
    session: String,
    file: Option<Mutex<File>>,
}

impl Log for Sink {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.filter.level_of(component_of(metadata.target()))
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format_event(self.format, SystemTime::now(), &self.session, record);
        // Un journal qui ne peut pas s'écrire ne doit jamais interrompre l'opération qu'il décrit.
        let _ = writeln!(io::stderr().lock(), "{line}");
        if let Some(file) = &self.file
            && let Ok(mut file) = file.lock()
        {
            let _ = writeln!(file, "{line}");
        }
    }

    fn flush(&self) {
        let _ = io::stderr().flush();
        if let Some(Ok(mut file)) = self.file.as_ref().map(Mutex::lock) {
            let _ = file.flush();
        }
    }
}

/// Réglages de journal tels que lus sur la ligne de commande et dans l'environnement.
#[derive(Debug)]
pub(crate) struct Settings {
    pub(crate) filter: Filter,
    pub(crate) format: LogFormat,
    pub(crate) file: Option<PathBuf>,
}

impl Settings {
    /// `default_format` vient de `bonk config` (`log.format`) ; `--log-format` l'emporte.
    pub(crate) fn from_args(
        args: &LogArgs,
        env_spec: Option<&str>,
        default_format: LogFormat,
    ) -> Result<Self, String> {
        Ok(Self {
            filter: if args.quiet {
                Filter::quiet(env_spec)?
            } else {
                Filter::new(args.verbose, env_spec)?
            },
            format: args.log_format.unwrap_or(default_format),
            file: args.log_file.clone(),
        })
    }
}

/// Installe le journal pour la durée du processus.
pub(crate) fn install(settings: Settings) -> Result<(), String> {
    let file = match &settings.file {
        Some(path) => Some(Mutex::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| format!("log: could not open {}: {e}", path.display()))?,
        )),
        None => None,
    };
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let max = settings.filter.max_level();
    let sink = Sink {
        filter: settings.filter,
        format: settings.format,
        session: format!("{:x}-{nanos:x}", std::process::id()),
        file,
    };
    log::set_boxed_logger(Box::new(sink)).map_err(|e| format!("log already installed: {e}"))?;
    log::set_max_level(max);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use log::Level;

    use super::*;

    fn at(secs: u64, millis: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(secs * 1000 + millis)
    }

    #[test]
    fn dates_utc() {
        assert_eq!(iso8601(at(0, 0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(at(1_700_000_000, 42)), "2023-11-14T22:13:20.042Z");
        // 29 février d'une année bissextile, et 1er mars qui suit.
        assert_eq!(iso8601(at(951_782_400, 0)), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso8601(at(951_868_800, 0)), "2000-03-01T00:00:00.000Z");
        assert_eq!(iso8601(at(4_102_444_799, 999)), "2099-12-31T23:59:59.999Z");
    }

    #[test]
    fn composants_d_apres_la_cible() {
        assert_eq!(component_of("bonk_gui::app"), "gui");
        assert_eq!(component_of("bonk_hal::linux"), "hal");
        assert_eq!(component_of("bonk_core::block"), "io");
        assert_eq!(component_of("bonk_cli::list"), "cli");
        assert_eq!(component_of("bonk::write"), "write");
        // Une cible « bonk::... » inconnue reste un événement de Bonk, pas du rendu.
        assert_eq!(component_of("bonk::inconnu"), "cli");
        assert_eq!(component_of("wgpu_hal::vulkan"), "render");
        assert_eq!(component_of("iced_winit"), "render");
    }

    #[test]
    fn niveaux_par_defaut_selon_v() -> Result<(), String> {
        let sans = Filter::new(0, None)?;
        assert_eq!(sans.level_of("hal"), LevelFilter::Warn);
        assert_eq!(sans.level_of("render"), LevelFilter::Error);
        let un = Filter::new(1, None)?;
        assert_eq!(
            (un.level_of("gui"), un.level_of("render")),
            (LevelFilter::Debug, LevelFilter::Info)
        );
        let deux = Filter::new(2, None)?;
        assert_eq!(
            (deux.level_of("io"), deux.level_of("render")),
            (LevelFilter::Trace, LevelFilter::Debug)
        );
        Ok(())
    }

    #[test]
    fn bonk_log_regle_chaque_composant() -> Result<(), String> {
        let filter = Filter::new(0, Some("gui=debug, hal=trace"))?;
        assert_eq!(filter.level_of("gui"), LevelFilter::Debug);
        assert_eq!(filter.level_of("hal"), LevelFilter::Trace);
        assert_eq!(filter.level_of("cli"), LevelFilter::Warn);
        assert_eq!(filter.max_level(), LevelFilter::Trace);
        // Un niveau seul vaut pour tous les composants, rendu compris ; un réglage précis l'emporte.
        let tout = Filter::new(0, Some("debug,render=warn"))?;
        assert_eq!(tout.level_of("io"), LevelFilter::Debug);
        assert_eq!(tout.level_of("render"), LevelFilter::Warn);
        Ok(())
    }

    #[test]
    fn bonk_log_invalide_est_refuse_avec_un_message_clair() {
        let erreur = Filter::new(0, Some("gui=bavard")).err().unwrap_or_default();
        assert!(erreur.contains("unknown log level"), "{erreur}");
        let erreur = Filter::new(0, Some("disque=debug"))
            .err()
            .unwrap_or_default();
        assert!(erreur.contains("unknown log component"), "{erreur}");
        assert!(erreur.contains("hal"), "{erreur}");
    }

    #[test]
    fn sans_reglage_rien_de_plus_que_les_avertissements() -> Result<(), String> {
        let filter = Filter::new(0, None)?;
        assert_eq!(filter.max_level(), LevelFilter::Warn);
        Ok(())
    }

    /// Construit un enregistrement le temps d'un appel : ses champs empruntent des données locales.
    fn avec_enregistrement<R>(
        args: std::fmt::Arguments<'_>,
        level: Level,
        target: &str,
        fields: &[(&str, &str)],
        f: impl FnOnce(&Record<'_>) -> R,
    ) -> R {
        let record = Record::builder()
            .args(args)
            .level(level)
            .target(target)
            .key_values(&fields)
            .build();
        f(&record)
    }

    #[test]
    fn evenement_au_format_texte() {
        let line = avec_enregistrement(
            format_args!("disk verdict"),
            Level::Debug,
            "bonk_hal::linux",
            &[("id", "usb-cle")],
            |rec| format_event(LogFormat::Text, at(1_700_000_000, 5), "s1", rec),
        );
        assert_eq!(
            line,
            "2023-11-14T22:13:20.005Z DEBUG hal    disk verdict id=\"usb-cle\""
        );
    }

    #[test]
    fn evenement_au_format_json() -> Result<(), serde_json::Error> {
        let line = avec_enregistrement(
            format_args!("click"),
            Level::Info,
            "bonk_gui::app",
            &[("id", "usb-cle")],
            |rec| format_event(LogFormat::Json, at(0, 0), "s1", rec),
        );
        let json: serde_json::Value = serde_json::from_str(&line)?;
        assert_eq!(json["ts"], "1970-01-01T00:00:00.000Z");
        assert_eq!(json["level"], "info");
        assert_eq!(json["component"], "gui");
        assert_eq!(json["session"], "s1");
        assert_eq!(json["message"], "click");
        assert_eq!(json["fields"]["id"], "usb-cle");
        assert!(!line.contains('\n'));
        Ok(())
    }

    #[test]
    fn un_message_multiligne_reste_sur_une_ligne_json() -> Result<(), serde_json::Error> {
        let line = avec_enregistrement(format_args!("a\nb"), Level::Warn, "bonk_cli", &[], |rec| {
            format_event(LogFormat::Json, at(0, 0), "s", rec)
        });
        assert!(!line.contains('\n'));
        let json: serde_json::Value = serde_json::from_str(&line)?;
        assert_eq!(json["message"], "a\nb");
        assert!(json.get("fields").is_none());
        Ok(())
    }
}
