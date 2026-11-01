//! Fichier image exposé comme périphérique bloc.
//!
//! Seuls les fichiers ordinaires sont acceptés : un périphérique (`/dev/sdX`, `\\.\PhysicalDriveN`), un
//! répertoire ou un tube est refusé **avant** toute ouverture, pour qu'une faute de frappe en test ne
//! puisse pas écrire sur un disque réel. Les disques réels passent par [`crate::BlockNode`], derrière le
//! filtre de sécurité.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

use bonk_core::{BlockDevice, BlockError, check_access, check_geometry};

/// Mode d'ouverture d'un fichier image existant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
}

/// Fichier image ordinaire utilisé comme [`BlockDevice`].
#[derive(Debug)]
pub struct FileDevice {
    file: File,
    size: u64,
    block_size: u32,
    read_only: bool,
}

impl FileDevice {
    /// Crée un fichier image creux de `size` octets, ouvert en lecture-écriture. Refuse d'écraser un
    /// fichier existant : supprimer l'ancien est un geste explicite de l'appelant.
    pub fn create(path: &Path, size: u64, block_size: u32) -> Result<Self, BlockError> {
        check_geometry(size, block_size)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        file.set_len(size)?;
        Ok(Self {
            file,
            size,
            block_size,
            read_only: false,
        })
    }

    /// Ouvre un fichier image existant, dont la taille doit être un multiple de `block_size`. Un chemin
    /// qui n'est pas un fichier ordinaire est refusé (`InvalidInput`).
    pub fn open(path: &Path, access: Access, block_size: u32) -> Result<Self, BlockError> {
        // Contrôle avant ouverture : ouvrir un tube bloquerait, ouvrir un périphérique est exactement
        // ce que ce type doit empêcher.
        ensure_regular_file(&fs::metadata(path)?)?;
        let read_only = access == Access::ReadOnly;
        let file = OpenOptions::new().read(true).write(!read_only).open(path)?;
        // Second contrôle sur le descripteur ouvert : le chemin a pu changer entre-temps.
        let metadata = file.metadata()?;
        ensure_regular_file(&metadata)?;
        let size = metadata.len();
        check_geometry(size, block_size)?;
        Ok(Self {
            file,
            size,
            block_size,
            read_only,
        })
    }
}

fn ensure_regular_file(metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.is_file() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the path is not a regular image file",
        ))
    }
}

impl BlockDevice for FileDevice {
    fn size(&self) -> u64 {
        self.size
    }

    fn block_size(&self) -> u32 {
        self.block_size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        check_access(self.size, self.block_size, offset, buf.len())?;
        read_exact_at(&self.file, buf, offset)?;
        Ok(())
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        if self.read_only {
            return Err(BlockError::ReadOnly);
        }
        check_access(self.size, self.block_size, offset, data.len())?;
        write_all_at(&self.file, data, offset)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        if !self.read_only {
            self.file.sync_all()?;
        }
        Ok(())
    }
}

// Accès positionnel : pas de curseur partagé, donc pas d'état à resynchroniser entre deux appels.

/// Lit au plus `buf.len()` octets à la position `offset`, sans curseur partagé entre les lecteurs d'un même
/// fichier.
#[cfg(unix)]
pub fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

/// Lit au plus `buf.len()` octets à la position `offset`. `seek_read` déplace aussi le curseur du fichier,
/// qu'aucun lecteur n'utilise.
#[cfg(windows)]
pub fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

/// Lit exactement `buf.len()` octets à la position `offset`.
#[cfg(unix)]
pub fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
}

#[cfg(unix)]
fn write_all_at(file: &File, data: &[u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, data, offset)
}

// Sous Windows, `seek_read` et `seek_write` peuvent transférer moins d'octets que demandé et
// déplacent le curseur du fichier (sans conséquence ici, il n'est jamais utilisé) : boucle explicite.

/// Lit exactement `buf.len()` octets à la position `offset`.
#[cfg(windows)]
pub fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        match file.seek_read(&mut buf[done..], offset + done as u64) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => done += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn write_all_at(file: &File, data: &[u8], offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < data.len() {
        match file.seek_write(&data[done..], offset + done as u64) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => done += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// Chemin temporaire unique, supprimé à la fin du test.
    struct TempImage(PathBuf);

    impl TempImage {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!("bonk-hal-{}-{n}-{label}.img", std::process::id());
            Self(std::env::temp_dir().join(name))
        }
    }

    impl Drop for TempImage {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| seed.wrapping_add((i % 251) as u8))
            .collect()
    }

    #[test]
    fn creation_donne_une_image_a_zero() -> Result<(), BlockError> {
        let img = TempImage::new("creation");
        let dev = FileDevice::create(&img.0, 16 * 512, 512)?;
        assert_eq!(dev.size(), 16 * 512);
        assert_eq!(dev.block_size(), 512);
        let mut buf = vec![0xAA; 16 * 512];
        dev.read_at(0, &mut buf)?;
        assert!(buf.iter().all(|&b| b == 0));
        assert_eq!(fs::metadata(&img.0)?.len(), 16 * 512);
        Ok(())
    }

    #[test]
    fn ecriture_persistee_apres_reouverture() -> Result<(), BlockError> {
        let img = TempImage::new("persistance");
        let data = pattern(3 * 512, 5);
        {
            let mut dev = FileDevice::create(&img.0, 16 * 512, 512)?;
            dev.write_at(4 * 512, &data)?;
            dev.flush()?;
        }
        let dev = FileDevice::open(&img.0, Access::ReadOnly, 512)?;
        let mut back = vec![0; 3 * 512];
        dev.read_at(4 * 512, &mut back)?;
        assert_eq!(back, data);
        Ok(())
    }

    #[test]
    fn lecture_seule_refuse_l_ecriture() -> Result<(), BlockError> {
        let img = TempImage::new("lecture-seule");
        FileDevice::create(&img.0, 4 * 512, 512)?;
        let mut dev = FileDevice::open(&img.0, Access::ReadOnly, 512)?;
        assert!(matches!(
            dev.write_at(0, &[1; 512]),
            Err(BlockError::ReadOnly)
        ));
        dev.flush()
    }

    #[test]
    fn lecture_ecriture_apres_reouverture() -> Result<(), BlockError> {
        let img = TempImage::new("lecture-ecriture");
        FileDevice::create(&img.0, 4 * 512, 512)?;
        let mut dev = FileDevice::open(&img.0, Access::ReadWrite, 512)?;
        dev.write_at(512, &[9; 512])?;
        let mut back = vec![0; 512];
        dev.read_at(512, &mut back)?;
        assert!(back.iter().all(|&b| b == 9));
        Ok(())
    }

    #[test]
    fn creation_refuse_d_ecraser_un_fichier() -> Result<(), BlockError> {
        let img = TempImage::new("existant");
        fs::write(&img.0, b"important")?;
        match FileDevice::create(&img.0, 4 * 512, 512) {
            Err(BlockError::Io(err)) => assert_eq!(err.kind(), io::ErrorKind::AlreadyExists),
            autre => panic!("attendu AlreadyExists, obtenu {autre:?}"),
        }
        assert_eq!(fs::read(&img.0)?, b"important");
        Ok(())
    }

    #[test]
    fn ouverture_refuse_un_repertoire() {
        match FileDevice::open(&std::env::temp_dir(), Access::ReadOnly, 512) {
            Err(BlockError::Io(err)) => assert_eq!(err.kind(), io::ErrorKind::InvalidInput),
            autre => panic!("attendu InvalidInput, obtenu {autre:?}"),
        }
    }

    /// `/dev/null` n'est ni un fichier ordinaire ni ouvert en lecture-écriture : preuve que la garde
    /// refuse un fichier spécial avant de l'ouvrir.
    #[cfg(unix)]
    #[test]
    fn ouverture_refuse_un_fichier_special() {
        match FileDevice::open(Path::new("/dev/null"), Access::ReadWrite, 512) {
            Err(BlockError::Io(err)) => assert_eq!(err.kind(), io::ErrorKind::InvalidInput),
            autre => panic!("attendu InvalidInput, obtenu {autre:?}"),
        }
    }

    #[test]
    fn ouverture_refuse_une_taille_non_multiple_du_bloc() -> Result<(), BlockError> {
        let img = TempImage::new("taille-impaire");
        fs::write(&img.0, vec![0; 1000])?;
        assert!(matches!(
            FileDevice::open(&img.0, Access::ReadOnly, 512),
            Err(BlockError::InvalidGeometry { .. })
        ));
        Ok(())
    }

    #[test]
    fn contrat_d_alignement_et_de_bornes() -> Result<(), BlockError> {
        let img = TempImage::new("contrat");
        let mut dev = FileDevice::create(&img.0, 4 * 512, 512)?;
        let mut buf = vec![0; 512];
        assert!(matches!(
            dev.read_at(7, &mut buf),
            Err(BlockError::Unaligned { .. })
        ));
        assert!(matches!(
            dev.write_at(4 * 512, &[1; 512]),
            Err(BlockError::OutOfRange { .. })
        ));
        // Aucune écriture partielle après un refus.
        let mut all = vec![0xFF; 4 * 512];
        dev.read_at(0, &mut all)?;
        assert!(all.iter().all(|&b| b == 0));
        Ok(())
    }

    #[test]
    fn bloc_de_4096_octets() -> Result<(), BlockError> {
        let img = TempImage::new("4k");
        let mut dev = FileDevice::create(&img.0, 4 * 4096, 4096)?;
        dev.write_at(4096, &pattern(4096, 3))?;
        assert!(matches!(
            dev.write_at(512, &pattern(512, 3)),
            Err(BlockError::Unaligned { .. })
        ));
        Ok(())
    }
}
