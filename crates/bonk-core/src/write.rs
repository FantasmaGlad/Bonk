//! Écriture brute d'une image sur un [`BlockDevice`] et vérification par relecture : l'empreinte
//! SHA-256 de l'image est calculée pendant l'écriture, puis comparée à celle du support relu. Rien
//! n'est écrit au-delà de la fin du périphérique, et la main revient à chaque bloc pour la
//! progression et l'annulation.

use std::fmt;
use std::io::{self, Read};
use std::ops::Range;
use std::time::Instant;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::block::{BlockDevice, BlockError};

/// 4 Mio, comme le `dd bs=4M` contre lequel le débit est mesuré.
pub const DEFAULT_CHUNK: usize = 4 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Téléchargement de l'image, avant tout geste sur le support.
    Download,
    /// En attente de l'autorisation du système.
    Authorize,
    /// Contrôle de l'empreinte de l'image, avant tout geste sur le support.
    Source,
    /// Écriture de l'image.
    Write,
    /// Vidage des tampons jusqu'au support.
    Flush,
    /// Relecture et comparaison.
    Verify,
}

/// Avancement d'une opération, rapporté à chaque bloc.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Progress {
    pub phase: Phase,
    pub bytes_done: u64,
    /// Total quand on le connaît (image brute, ou écriture déjà faite pour la vérification).
    pub bytes_total: Option<u64>,
    pub bytes_per_second: f64,
    pub elapsed_ms: u64,
}

/// Réponse du rappel de progression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Continue,
    Cancel,
}

#[derive(Debug)]
pub enum WriteError {
    Device(BlockError),
    /// Lecture de l'image impossible (fichier illisible, flux compressé corrompu).
    Source(io::Error),
    /// L'image dépasse le périphérique ; rien n'a été écrit au-delà de sa fin.
    TooLarge {
        device_size: u64,
        needed: Option<u64>,
    },
    Cancelled {
        bytes_written: u64,
    },
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(err) => write!(f, "écriture impossible : {err}"),
            Self::Source(err) => write!(f, "image illisible : {err}"),
            Self::TooLarge {
                device_size,
                needed: Some(needed),
            } => write!(
                f,
                "l'image fait {needed} octets, le support {device_size} octets"
            ),
            Self::TooLarge { device_size, .. } => write!(
                f,
                "l'image décompressée dépasse le support ({device_size} octets) : écriture arrêtée à sa fin"
            ),
            Self::Cancelled { bytes_written } => {
                write!(f, "annulée après {bytes_written} octets écrits")
            }
        }
    }
}

impl std::error::Error for WriteError {}

impl From<BlockError> for WriteError {
    fn from(err: BlockError) -> Self {
        Self::Device(err)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct WriteReport {
    /// Octets de l'image écrits.
    pub bytes: u64,
    /// Octets réellement envoyés au support : l'image arrondie au secteur, le complément est à zéro.
    pub padded_bytes: u64,
    /// SHA-256 des octets de l'image, en hexadécimal.
    pub sha256: String,
    pub elapsed_ms: u64,
    pub bytes_per_second: f64,
}

#[derive(Debug)]
pub enum VerifyError {
    Device(BlockError),
    Source(io::Error),
    Mismatch { expected: String, actual: String },
    Cancelled,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(err) => write!(f, "relecture impossible : {err}"),
            Self::Source(err) => write!(f, "image illisible : {err}"),
            Self::Mismatch { expected, actual } => write!(
                f,
                "les octets relus ne correspondent pas à l'image (SHA-256 attendu {expected}, relu {actual})"
            ),
            Self::Cancelled => write!(f, "vérification annulée"),
        }
    }
}

impl std::error::Error for VerifyError {}

impl From<BlockError> for VerifyError {
    fn from(err: BlockError) -> Self {
        Self::Device(err)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct VerifyReport {
    pub bytes: u64,
    /// Le cache du système a été vidé avant la relecture. Faux : la relecture a pu lire la mémoire du
    /// système et non le support.
    pub cache_dropped: bool,
    pub sha256: String,
    pub elapsed_ms: u64,
    pub bytes_per_second: f64,
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn rate(bytes: u64, started: Instant) -> f64 {
    let secs = started.elapsed().as_secs_f64();
    if secs > 0.0 { bytes as f64 / secs } else { 0.0 }
}

fn millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn progress_at(phase: Phase, done: u64, total: Option<u64>, started: Instant) -> Progress {
    Progress {
        phase,
        bytes_done: done,
        bytes_total: total,
        bytes_per_second: rate(done, started),
        elapsed_ms: millis(started),
    }
}

fn chunk_for(chunk: usize, block: u32) -> usize {
    let block = block as usize;
    chunk.max(block).div_ceil(block) * block
}

fn read_full(src: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match src.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    Ok(filled)
}

/// Écrit `src` au début de `dev`. Si `size_hint` dépasse le support, rien n'est touché.
///
/// # Errors
/// Sur annulation ou erreur d'entrée-sortie, le support est dans un état partiel et doit être
/// réécrit en entier.
pub fn write_image(
    src: &mut dyn Read,
    dev: &mut dyn BlockDevice,
    size_hint: Option<u64>,
    chunk: usize,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<WriteReport, WriteError> {
    if let Some(needed) = size_hint
        && needed > dev.size()
    {
        return Err(WriteError::TooLarge {
            device_size: dev.size(),
            needed: Some(needed),
        });
    }
    let block = u64::from(dev.block_size());
    let mut buf = vec![0u8; chunk_for(chunk, dev.block_size())];
    let mut hasher = Sha256::new();
    let started = Instant::now();
    let (mut written, mut offset) = (0u64, 0u64);

    if progress(&progress_at(Phase::Write, 0, size_hint, started)) == Control::Cancel {
        return Err(WriteError::Cancelled { bytes_written: 0 });
    }
    loop {
        let n = read_full(src, &mut buf).map_err(WriteError::Source)?;
        if n == 0 {
            break;
        }
        let padded = (n as u64).div_ceil(block) * block;
        buf[n..usize::try_from(padded).unwrap_or(n)].fill(0);
        if offset + padded > dev.size() {
            return Err(WriteError::TooLarge {
                device_size: dev.size(),
                needed: None,
            });
        }
        dev.write_at(offset, &buf[..usize::try_from(padded).unwrap_or(n)])?;
        hasher.update(&buf[..n]);
        written += n as u64;
        offset += padded;
        if progress(&progress_at(Phase::Write, written, size_hint, started)) == Control::Cancel {
            return Err(WriteError::Cancelled {
                bytes_written: written,
            });
        }
    }
    let _ = progress(&progress_at(Phase::Flush, written, Some(written), started));
    dev.flush()?;
    Ok(WriteReport {
        bytes: written,
        padded_bytes: offset,
        sha256: hex(&hasher.finalize()),
        elapsed_ms: millis(started),
        bytes_per_second: rate(written, started),
    })
}

/// Vide le cache, puis relit les `len` premiers octets de `dev` et compare leur SHA-256 à `expected`.
pub fn verify(
    dev: &mut dyn BlockDevice,
    len: u64,
    expected_sha256: &str,
    chunk: usize,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<VerifyReport, VerifyError> {
    verify_ignoring(dev, len, expected_sha256, chunk, &[], progress)
}

fn zero_ignored(buf: &mut [u8], offset: u64, ignore: &[Range<u64>]) {
    let end = offset + buf.len() as u64;
    for range in ignore {
        let (from, to) = (range.start.max(offset), range.end.min(end));
        if from < to {
            let (from, to) = (from - offset, to - offset);
            buf[usize::try_from(from).unwrap_or(0)..usize::try_from(to).unwrap_or(0)].fill(0);
        }
    }
}

/// Comme [`verify`], avec les octets des plages `ignore` mis à zéro des deux côtés avant l'empreinte :
/// la réparation du GPT réécrit légitimement quelques champs.
pub fn verify_ignoring(
    dev: &mut dyn BlockDevice,
    len: u64,
    expected_sha256: &str,
    chunk: usize,
    ignore: &[Range<u64>],
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<VerifyReport, VerifyError> {
    let cache_dropped = dev.drop_cache().is_ok();
    let block = u64::from(dev.block_size());
    let step = chunk_for(chunk, dev.block_size());
    let mut buf = vec![0u8; step];
    let mut hasher = Sha256::new();
    let started = Instant::now();
    let mut done = 0u64;
    while done < len {
        let n = usize::try_from((len - done).min(step as u64)).unwrap_or(step);
        let padded = usize::try_from((n as u64).div_ceil(block) * block).unwrap_or(n);
        dev.read_at(done, &mut buf[..padded])?;
        zero_ignored(&mut buf[..n], done, ignore);
        hasher.update(&buf[..n]);
        done += n as u64;
        if progress(&progress_at(Phase::Verify, done, Some(len), started)) == Control::Cancel {
            return Err(VerifyError::Cancelled);
        }
    }
    let actual = hex(&hasher.finalize());
    if actual.eq_ignore_ascii_case(expected_sha256) {
        Ok(VerifyReport {
            bytes: len,
            cache_dropped,
            sha256: actual,
            elapsed_ms: millis(started),
            bytes_per_second: rate(len, started),
        })
    } else {
        Err(VerifyError::Mismatch {
            expected: expected_sha256.to_owned(),
            actual,
        })
    }
}

/// Longueur et SHA-256 de `src`, sans rien écrire, avec les octets des plages `ignore` mis à zéro avant
/// l'empreinte : pour comparer un support à une image.
pub fn hash_source_ignoring(
    src: &mut dyn Read,
    size_hint: Option<u64>,
    chunk: usize,
    ignore: &[Range<u64>],
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<(u64, String), VerifyError> {
    hash_reader(src, size_hint, chunk, ignore, Phase::Verify, progress)
}

fn hash_reader(
    src: &mut dyn Read,
    size_hint: Option<u64>,
    chunk: usize,
    ignore: &[Range<u64>],
    phase: Phase,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<(u64, String), VerifyError> {
    let mut buf = vec![0u8; chunk.max(4096)];
    let mut hasher = Sha256::new();
    let started = Instant::now();
    let mut total = 0u64;
    loop {
        let n = read_full(src, &mut buf).map_err(VerifyError::Source)?;
        if n == 0 {
            break;
        }
        zero_ignored(&mut buf[..n], total, ignore);
        hasher.update(&buf[..n]);
        total += n as u64;
        if progress(&progress_at(phase, total, size_hint, started)) == Control::Cancel {
            return Err(VerifyError::Cancelled);
        }
    }
    Ok((total, hex(&hasher.finalize())))
}

/// SHA-256 du flux tel quel, compressé ou non : c'est sur le fichier téléchargé que les éditeurs
/// publient leur empreinte.
pub fn hash_stream(
    src: &mut dyn Read,
    total: Option<u64>,
    chunk: usize,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<String, VerifyError> {
    hash_reader(src, total, chunk, &[], Phase::Source, progress).map(|(_, sha256)| sha256)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::block::MemoryDevice;

    fn data(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 253) as u8).collect()
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    fn silent() -> impl FnMut(&Progress) -> Control {
        |_| Control::Continue
    }

    #[test]
    fn ecriture_puis_verification_d_une_image_multiple_du_secteur()
    -> Result<(), Box<dyn std::error::Error>> {
        let image = data(8 * 512);
        let mut dev = MemoryDevice::new(16 * 512, 512)?;
        let report = write_image(
            &mut Cursor::new(image.clone()),
            &mut dev,
            Some(image.len() as u64),
            1024,
            &mut silent(),
        )?;
        assert_eq!((report.bytes, report.padded_bytes), (4096, 4096));
        assert_eq!(report.sha256, sha(&image));
        assert_eq!(&dev.as_bytes()[..4096], image.as_slice());
        assert!(dev.as_bytes()[4096..].iter().all(|&b| b == 0));
        let verified = verify(&mut dev, report.bytes, &report.sha256, 1024, &mut silent())?;
        assert_eq!(verified.sha256, report.sha256);
        Ok(())
    }

    #[test]
    fn la_derniere_partie_est_completee_a_zero_sans_changer_l_empreinte()
    -> Result<(), Box<dyn std::error::Error>> {
        let image = data(1000); // pas un multiple de 512
        let mut dev = MemoryDevice::new(8 * 512, 512)?;
        let report = write_image(
            &mut Cursor::new(image.clone()),
            &mut dev,
            None,
            4096,
            &mut silent(),
        )?;
        assert_eq!((report.bytes, report.padded_bytes), (1000, 1024));
        assert_eq!(report.sha256, sha(&image));
        assert_eq!(&dev.as_bytes()[..1000], image.as_slice());
        assert!(dev.as_bytes()[1000..1024].iter().all(|&b| b == 0));
        verify(&mut dev, 1000, &report.sha256, 4096, &mut silent())?;
        Ok(())
    }

    #[test]
    fn secteurs_de_4096_octets_et_blocs_plus_petits_que_le_secteur()
    -> Result<(), Box<dyn std::error::Error>> {
        let image = data(10_000);
        let mut dev = MemoryDevice::new(8 * 4096, 4096)?;
        // Un bloc demandé de 100 octets est relevé au secteur.
        let report = write_image(
            &mut Cursor::new(image.clone()),
            &mut dev,
            None,
            100,
            &mut silent(),
        )?;
        assert_eq!((report.bytes, report.padded_bytes), (10_000, 12_288));
        verify(&mut dev, report.bytes, &report.sha256, 100, &mut silent())?;
        Ok(())
    }

    #[test]
    fn image_connue_trop_grande_refusee_sans_rien_ecrire() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut dev = MemoryDevice::new(4 * 512, 512)?;
        let image = data(5000);
        let error = write_image(
            &mut Cursor::new(image),
            &mut dev,
            Some(5000),
            512,
            &mut silent(),
        )
        .err();
        assert!(matches!(
            error,
            Some(WriteError::TooLarge {
                device_size: 2048,
                needed: Some(5000)
            })
        ));
        assert!(dev.as_bytes().iter().all(|&b| b == 0));
        Ok(())
    }

    #[test]
    fn image_de_taille_inconnue_qui_deborde_s_arrete_a_la_fin_du_support()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(4 * 512, 512)?;
        let image = data(5000);
        let error = write_image(
            &mut Cursor::new(image.clone()),
            &mut dev,
            None,
            512,
            &mut silent(),
        )
        .err();
        assert!(matches!(
            error,
            Some(WriteError::TooLarge { needed: None, .. })
        ));
        // Ce qui tenait a été écrit, rien au-delà (le support n'a que 2048 octets).
        assert_eq!(dev.as_bytes(), &image[..2048]);
        Ok(())
    }

    #[test]
    fn annulation_entre_deux_blocs() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(16 * 512, 512)?;
        let mut calls = 0;
        let mut cancel_after_two = |_: &Progress| {
            calls += 1;
            if calls > 2 {
                Control::Cancel
            } else {
                Control::Continue
            }
        };
        let error = write_image(
            &mut Cursor::new(data(8 * 512)),
            &mut dev,
            None,
            512,
            &mut cancel_after_two,
        )
        .err();
        assert!(matches!(
            error,
            Some(WriteError::Cancelled {
                bytes_written: 1024
            })
        ));
        Ok(())
    }

    #[test]
    fn progression_croissante_avec_phases() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(16 * 512, 512)?;
        let mut seen = Vec::new();
        let mut record = |p: &Progress| {
            seen.push((p.phase, p.bytes_done));
            Control::Continue
        };
        write_image(
            &mut Cursor::new(data(4 * 512)),
            &mut dev,
            Some(2048),
            512,
            &mut record,
        )?;
        assert_eq!(seen.first(), Some(&(Phase::Write, 0)));
        assert_eq!(seen.last(), Some(&(Phase::Flush, 2048)));
        let writes: Vec<u64> = seen
            .iter()
            .filter(|(p, _)| *p == Phase::Write)
            .map(|(_, b)| *b)
            .collect();
        assert!(writes.windows(2).all(|w| w[0] <= w[1]), "{writes:?}");
        assert_eq!(writes.last(), Some(&2048));
        Ok(())
    }

    #[test]
    fn la_verification_detecte_un_octet_different() -> Result<(), Box<dyn std::error::Error>> {
        let image = data(8 * 512);
        let mut dev = MemoryDevice::new(16 * 512, 512)?;
        let report = write_image(&mut Cursor::new(image), &mut dev, None, 1024, &mut silent())?;
        let mut tampered = dev.as_bytes().to_vec();
        tampered[3000] ^= 0x01;
        let mut bad = MemoryDevice::from_bytes(tampered, 512)?;
        let error = verify(&mut bad, report.bytes, &report.sha256, 1024, &mut silent()).err();
        assert!(
            matches!(error, Some(VerifyError::Mismatch { ref expected, ref actual }) if *expected == report.sha256 && actual != expected)
        );
        // Même taille, mais une longueur plus courte que ce qui a été écrit ne correspond pas non plus.
        assert!(
            verify(
                &mut dev,
                report.bytes - 512,
                &report.sha256,
                1024,
                &mut silent()
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn hash_source_et_comparaison_sans_ecriture() -> Result<(), Box<dyn std::error::Error>> {
        let image = data(3000);
        let (len, digest) = hash_source_ignoring(
            &mut Cursor::new(image.clone()),
            Some(3000),
            700,
            &[],
            &mut silent(),
        )?;
        assert_eq!((len, digest.as_str()), (3000, sha(&image).as_str()));
        let mut dev = MemoryDevice::new(8 * 512, 512)?;
        write_image(&mut Cursor::new(image), &mut dev, None, 512, &mut silent())?;
        verify(&mut dev, len, &digest, 512, &mut silent())?;
        Ok(())
    }

    #[test]
    fn erreur_de_lecture_de_la_source() -> Result<(), Box<dyn std::error::Error>> {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::InvalidData, "flux corrompu"))
            }
        }
        let mut dev = MemoryDevice::new(4 * 512, 512)?;
        let error = write_image(&mut Broken, &mut dev, None, 512, &mut silent()).err();
        assert!(matches!(error, Some(WriteError::Source(_))));
        Ok(())
    }

    /// Un cache que le système refuse de vider est dit dans le rapport : la relecture a pu lire la
    /// mémoire et non le support.
    #[test]
    fn la_relecture_dit_si_le_cache_a_ete_vide() -> Result<(), Box<dyn std::error::Error>> {
        struct Sticky(MemoryDevice);
        impl BlockDevice for Sticky {
            fn size(&self) -> u64 {
                self.0.size()
            }
            fn block_size(&self) -> u32 {
                self.0.block_size()
            }
            fn read_at(&self, o: u64, b: &mut [u8]) -> Result<(), BlockError> {
                self.0.read_at(o, b)
            }
            fn write_at(&mut self, o: u64, d: &[u8]) -> Result<(), BlockError> {
                self.0.write_at(o, d)
            }
            fn flush(&mut self) -> Result<(), BlockError> {
                Ok(())
            }
            fn drop_cache(&mut self) -> Result<(), BlockError> {
                Err(BlockError::Io(io::Error::other("refusé")))
            }
        }
        let image = data(2048);
        let mut dev = MemoryDevice::new(4096, 512)?;
        let report = write_image(
            &mut Cursor::new(image.clone()),
            &mut dev,
            None,
            512,
            &mut silent(),
        )?;
        assert!(verify(&mut dev, report.bytes, &report.sha256, 512, &mut silent())?.cache_dropped);
        let mut sticky = Sticky(dev);
        let verified = verify(
            &mut sticky,
            report.bytes,
            &report.sha256,
            512,
            &mut silent(),
        )?;
        assert!(!verified.cache_dropped);
        Ok(())
    }

    #[test]
    fn ecriture_sur_un_peripherique_en_lecture_seule_est_une_erreur()
    -> Result<(), Box<dyn std::error::Error>> {
        struct ReadOnly(MemoryDevice);
        impl BlockDevice for ReadOnly {
            fn size(&self) -> u64 {
                self.0.size()
            }
            fn block_size(&self) -> u32 {
                self.0.block_size()
            }
            fn read_at(&self, o: u64, b: &mut [u8]) -> Result<(), BlockError> {
                self.0.read_at(o, b)
            }
            fn write_at(&mut self, _: u64, _: &[u8]) -> Result<(), BlockError> {
                Err(BlockError::ReadOnly)
            }
            fn flush(&mut self) -> Result<(), BlockError> {
                Ok(())
            }
        }
        let mut dev = ReadOnly(MemoryDevice::new(4 * 512, 512)?);
        let error = write_image(
            &mut Cursor::new(data(512)),
            &mut dev,
            None,
            512,
            &mut silent(),
        )
        .err();
        assert!(matches!(
            error,
            Some(WriteError::Device(BlockError::ReadOnly))
        ));
        Ok(())
    }

    #[test]
    fn les_plages_ignorees_sont_exclues_des_deux_cotes_et_le_reste_est_compare()
    -> Result<(), Box<dyn std::error::Error>> {
        let image: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let ignore = [10..20u64, 4090..4096];
        let (len, sha) = hash_source_ignoring(
            &mut Cursor::new(image.clone()),
            None,
            1024,
            &ignore,
            &mut |_| Control::Continue,
        )?;
        assert_eq!(len, 4096);
        // Le support diffère dans les plages ignorées (y compris à cheval sur deux blocs de lecture).
        let mut other = image.clone();
        other[12] ^= 0xFF;
        other[4092] ^= 0xFF;
        let mut dev = MemoryDevice::from_bytes(other.clone(), 512)?;
        assert!(
            verify_ignoring(&mut dev, len, &sha, 1024, &ignore, &mut |_| {
                Control::Continue
            })
            .is_ok()
        );
        // Sans ignorer, la même différence est détectée ; un octet hors plage l'est aussi.
        assert!(verify(&mut dev, len, &sha, 1024, &mut |_| Control::Continue).is_err());
        other[100] ^= 1;
        let mut dev = MemoryDevice::from_bytes(other, 512)?;
        assert!(matches!(
            verify_ignoring(&mut dev, len, &sha, 1024, &ignore, &mut |_| {
                Control::Continue
            }),
            Err(VerifyError::Mismatch { .. })
        ));
        Ok(())
    }
}
