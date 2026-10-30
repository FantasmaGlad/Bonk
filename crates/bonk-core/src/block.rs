//! Abstraction de périphérique bloc.
//!
//! Tout le code de `bonk-core` (partitionnement, écriture, vérification) lit et écrit à travers le
//! trait [`BlockDevice`]. Les implémentations qui touchent le système (fichier image, disque brut)
//! vivent dans `bonk-hal` ; [`MemoryDevice`] sert aux tests purs.
//!
//! # Contrat
//!
//! Les accès sont **positionnels** (décalage absolu en octets, pas de curseur partagé) et **alignés** :
//! le décalage et la longueur sont des multiples de [`BlockDevice::block_size`]. Un fichier image
//! accepterait n'importe quel accès, mais un périphérique brut (`O_DIRECT`, `\\.\PhysicalDriveN`,
//! `/dev/rdiskN`) refuse les accès non alignés : imposer l'alignement partout révèle l'erreur en test
//! au lieu de la révéler sur du matériel. Les accès hors des bornes sont refusés, jamais tronqués.

use std::error::Error;
use std::fmt;
use std::io;

/// Taille de secteur logique par défaut, en octets.
pub const DEFAULT_BLOCK_SIZE: u32 = 512;

/// Échec d'un accès à un périphérique bloc.
#[derive(Debug)]
pub enum BlockError {
    /// La taille de bloc ou la taille du périphérique est invalide.
    InvalidGeometry { size: u64, block_size: u32 },
    /// Le décalage ou la longueur n'est pas un multiple de la taille de bloc.
    Unaligned {
        offset: u64,
        len: u64,
        block_size: u32,
    },
    /// L'accès dépasse la fin du périphérique.
    OutOfRange { offset: u64, len: u64, size: u64 },
    /// Écriture demandée sur un périphérique ouvert en lecture seule.
    ReadOnly,
    /// Erreur d'entrée-sortie de l'implémentation sous-jacente.
    Io(io::Error),
}

impl fmt::Display for BlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGeometry { size, block_size } => write!(
                f,
                "géométrie invalide : taille {size} octets, bloc de {block_size} octets \
                 (le bloc doit être une puissance de deux d'au moins {DEFAULT_BLOCK_SIZE} \
                 et diviser la taille)"
            ),
            Self::Unaligned {
                offset,
                len,
                block_size,
            } => write!(
                f,
                "accès non aligné : décalage {offset}, longueur {len}, bloc de {block_size} octets"
            ),
            Self::OutOfRange { offset, len, size } => write!(
                f,
                "accès hors des bornes : décalage {offset}, longueur {len}, taille {size} octets"
            ),
            Self::ReadOnly => write!(f, "écriture refusée : périphérique en lecture seule"),
            Self::Io(err) => write!(f, "erreur d'entrée-sortie : {err}"),
        }
    }
}

impl Error for BlockError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for BlockError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Périphérique adressable par blocs, sans curseur.
pub trait BlockDevice {
    /// Taille totale en octets, multiple de [`Self::block_size`].
    fn size(&self) -> u64;

    /// Taille de bloc logique en octets (puissance de deux, au moins 512).
    fn block_size(&self) -> u32;

    /// Remplit `buf` avec les octets à partir de `offset`.
    ///
    /// # Errors
    /// [`BlockError::Unaligned`], [`BlockError::OutOfRange`] ou [`BlockError::Io`].
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError>;

    /// Écrit `data` à partir de `offset`. En cas d'erreur de validation, rien n'est écrit.
    ///
    /// # Errors
    /// [`BlockError::ReadOnly`], [`BlockError::Unaligned`], [`BlockError::OutOfRange`] ou
    /// [`BlockError::Io`].
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError>;

    /// Force les écritures en attente jusqu'au support.
    ///
    /// # Errors
    /// [`BlockError::Io`].
    fn flush(&mut self) -> Result<(), BlockError>;
}

/// Vérifie qu'une géométrie est utilisable : bloc en puissance de deux d'au moins 512 octets, taille
/// multiple du bloc.
///
/// # Errors
/// [`BlockError::InvalidGeometry`].
pub fn check_geometry(size: u64, block_size: u32) -> Result<(), BlockError> {
    let valid = block_size >= DEFAULT_BLOCK_SIZE
        && block_size.is_power_of_two()
        && size.is_multiple_of(u64::from(block_size));
    if valid {
        Ok(())
    } else {
        Err(BlockError::InvalidGeometry { size, block_size })
    }
}

/// Valide un accès de `len` octets à `offset` : alignement, puis bornes.
///
/// Toute implémentation de [`BlockDevice`] l'appelle avant de toucher au support, pour que le contrat
/// soit le même partout.
///
/// # Errors
/// [`BlockError::Unaligned`] ou [`BlockError::OutOfRange`] (y compris en cas de dépassement
/// arithmétique de `offset + len`).
pub fn check_access(size: u64, block_size: u32, offset: u64, len: usize) -> Result<(), BlockError> {
    let len = len as u64;
    let block = u64::from(block_size);
    if !offset.is_multiple_of(block) || !len.is_multiple_of(block) {
        return Err(BlockError::Unaligned {
            offset,
            len,
            block_size,
        });
    }
    match offset.checked_add(len) {
        Some(end) if end <= size => Ok(()),
        _ => Err(BlockError::OutOfRange { offset, len, size }),
    }
}

/// Périphérique en mémoire, initialisé à zéro : l'implémentation de référence pour les tests purs.
#[derive(Debug, Clone)]
pub struct MemoryDevice {
    data: Vec<u8>,
    block_size: u32,
}

impl MemoryDevice {
    /// Crée un périphérique de `size` octets remplis de zéros.
    ///
    /// # Errors
    /// [`BlockError::InvalidGeometry`], y compris quand `size` ne tient pas en mémoire adressable.
    pub fn new(size: u64, block_size: u32) -> Result<Self, BlockError> {
        check_geometry(size, block_size)?;
        let len =
            usize::try_from(size).map_err(|_| BlockError::InvalidGeometry { size, block_size })?;
        Ok(Self {
            data: vec![0; len],
            block_size,
        })
    }

    /// Crée un périphérique qui contient déjà `data`.
    ///
    /// # Errors
    /// [`BlockError::InvalidGeometry`].
    pub fn from_bytes(data: Vec<u8>, block_size: u32) -> Result<Self, BlockError> {
        check_geometry(data.len() as u64, block_size)?;
        Ok(Self { data, block_size })
    }

    /// Contenu complet, pour les assertions des tests.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
}

impl BlockDevice for MemoryDevice {
    fn size(&self) -> u64 {
        self.data.len() as u64
    }

    fn block_size(&self) -> u32 {
        self.block_size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        check_access(self.size(), self.block_size, offset, buf.len())?;
        // Les bornes sont validées : `offset + len <= size`, donc l'indexation ne peut pas échouer.
        let start = offset as usize;
        buf.copy_from_slice(&self.data[start..start + buf.len()]);
        Ok(())
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        check_access(self.size(), self.block_size, offset, data.len())?;
        let start = offset as usize;
        self.data[start..start + data.len()].copy_from_slice(data);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| seed.wrapping_add((i % 251) as u8))
            .collect()
    }

    #[test]
    fn geometrie_acceptee() {
        assert!(check_geometry(0, 512).is_ok());
        assert!(check_geometry(1024 * 1024, 512).is_ok());
        assert!(check_geometry(1024 * 1024, 4096).is_ok());
    }

    #[test]
    fn geometrie_refusee() {
        for (size, block) in [
            (4096, 0),
            (4096, 256),
            (4096, 513),
            (4096, 1536),
            (1000, 512),
            (4096 + 512, 4096),
        ] {
            assert!(
                matches!(
                    check_geometry(size, block),
                    Err(BlockError::InvalidGeometry { .. })
                ),
                "{size} octets, bloc {block}"
            );
        }
    }

    #[test]
    fn peripherique_neuf_est_a_zero() -> Result<(), BlockError> {
        let dev = MemoryDevice::new(4096, 512)?;
        assert_eq!(dev.size(), 4096);
        assert_eq!(dev.block_size(), 512);
        let mut buf = vec![0xAA; 4096];
        dev.read_at(0, &mut buf)?;
        assert!(buf.iter().all(|&b| b == 0));
        Ok(())
    }

    #[test]
    fn ecriture_puis_lecture_alignees() -> Result<(), BlockError> {
        let mut dev = MemoryDevice::new(8 * 512, 512)?;
        let data = pattern(2 * 512, 7);
        dev.write_at(3 * 512, &data)?;
        let mut back = vec![0; 2 * 512];
        dev.read_at(3 * 512, &mut back)?;
        assert_eq!(back, data);
        // Les blocs voisins sont intacts.
        let mut before = vec![0xFF; 512];
        dev.read_at(2 * 512, &mut before)?;
        assert!(before.iter().all(|&b| b == 0));
        dev.flush()
    }

    #[test]
    fn bloc_de_4096_octets() -> Result<(), BlockError> {
        let mut dev = MemoryDevice::new(4 * 4096, 4096)?;
        dev.write_at(4096, &pattern(4096, 1))?;
        assert!(matches!(
            dev.write_at(512, &pattern(512, 1)),
            Err(BlockError::Unaligned { .. })
        ));
        Ok(())
    }

    #[test]
    fn acces_non_aligne_refuse_sans_rien_ecrire() -> Result<(), BlockError> {
        let mut dev = MemoryDevice::new(4 * 512, 512)?;
        let mut buf = vec![0; 512];
        assert!(matches!(
            dev.read_at(1, &mut buf),
            Err(BlockError::Unaligned { offset: 1, .. })
        ));
        assert!(matches!(
            dev.write_at(0, &[1; 100]),
            Err(BlockError::Unaligned { len: 100, .. })
        ));
        assert!(dev.as_bytes().iter().all(|&b| b == 0));
        Ok(())
    }

    #[test]
    fn acces_hors_bornes_refuse_sans_rien_ecrire() -> Result<(), BlockError> {
        let mut dev = MemoryDevice::new(4 * 512, 512)?;
        let mut buf = vec![0; 2 * 512];
        assert!(matches!(
            dev.read_at(3 * 512, &mut buf),
            Err(BlockError::OutOfRange { .. })
        ));
        assert!(matches!(
            dev.write_at(4 * 512, &[1; 512]),
            Err(BlockError::OutOfRange { .. })
        ));
        // Dépassement arithmétique de `offset + len`.
        assert!(matches!(
            dev.write_at(u64::MAX - 511, &[1; 1024]),
            Err(BlockError::OutOfRange { .. })
        ));
        assert!(dev.as_bytes().iter().all(|&b| b == 0));
        Ok(())
    }

    #[test]
    fn acces_vide_a_la_fin_est_permis() -> Result<(), BlockError> {
        let mut dev = MemoryDevice::new(2 * 512, 512)?;
        dev.read_at(2 * 512, &mut [])?;
        dev.write_at(2 * 512, &[])
    }

    #[test]
    fn from_bytes_conserve_le_contenu() -> Result<(), BlockError> {
        let data = pattern(2 * 512, 9);
        let dev = MemoryDevice::from_bytes(data.clone(), 512)?;
        assert_eq!(dev.as_bytes(), data.as_slice());
        assert!(MemoryDevice::from_bytes(vec![0; 700], 512).is_err());
        Ok(())
    }
}
