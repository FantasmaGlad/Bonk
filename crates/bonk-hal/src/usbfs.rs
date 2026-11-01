//! Transport `usbfs` du stockage de masse : parle à la clé depuis l'espace utilisateur, sans le pilote du
//! noyau. Sous Linux, il sert de banc d'essai sur une vraie clé. Sous Android, sans root, le seul accès au
//! matériel est le descripteur que rend `UsbDeviceConnection.getFileDescriptor()` : les mêmes appels y valent.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::path::Path;

use bonk_core::msc::{BulkError, BulkTransport, Direction};

/// Sans réponse dans ce délai, la clé est tenue pour perdue. Vider son cache peut prendre longtemps.
const TIMEOUT_MS: u32 = 60_000;

/// Taille d'un transfert. Le noyau réserve son tampon d'un seul bloc : sur un téléphone dont la mémoire est
/// fragmentée, 1 Mio finit par être refusé alors qu'il reste de la place, et l'écriture s'arrêtait là.
const TRANSFER_START: usize = 256 << 10;
const TRANSFER_MIN: usize = 16 << 10;

const CLASS_STORAGE: u8 = 0x08;
const SUBCLASS_SCSI: u8 = 0x06;
const PROTOCOL_BULK_ONLY: u8 = 0x50;
const DESCRIPTOR_INTERFACE: u8 = 4;
const DESCRIPTOR_ENDPOINT: u8 = 5;
const ATTRIBUTES_BULK: u8 = 2;
const ENDPOINT_IN: u8 = 0x80;
const REQUEST_MASS_STORAGE_RESET: u8 = 0xFF;
const REQUEST_TYPE_CLASS_INTERFACE_OUT: u8 = 0x21;

/// L'interface de stockage trouvée dans les descripteurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageInterface {
    pub number: u8,
    pub endpoint_in: u8,
    pub endpoint_out: u8,
}

/// Première interface SCSI « bulk-only ». Une clé qui n'expose qu'UAS n'est pas prise en charge. Des
/// descripteurs tronqués ne donnent rien plutôt qu'une lecture hors bornes.
#[must_use]
pub(crate) fn find_storage_interface(descriptors: &[u8]) -> Option<StorageInterface> {
    let mut current: Option<u8> = None;
    let (mut endpoint_in, mut endpoint_out) = (None, None);
    let mut rest = descriptors;
    while let [length, kind, ..] = *rest {
        let length = usize::from(length);
        if length < 2 || length > rest.len() {
            return None;
        }
        let descriptor = &rest[..length];
        match (kind, descriptor) {
            (DESCRIPTOR_INTERFACE, [_, _, number, _, _, class, subclass, protocol, ..]) => {
                if current.is_some() {
                    break;
                }
                let wanted = (*class, *subclass, *protocol)
                    == (CLASS_STORAGE, SUBCLASS_SCSI, PROTOCOL_BULK_ONLY);
                current = wanted.then_some(*number);
            }
            (DESCRIPTOR_ENDPOINT, [_, _, address, attributes, ..])
                if current.is_some() && attributes & 3 == ATTRIBUTES_BULK =>
            {
                if address & ENDPOINT_IN != 0 {
                    endpoint_in = Some(*address);
                } else {
                    endpoint_out = Some(*address);
                }
            }
            _ => {}
        }
        rest = &rest[length..];
    }
    Some(StorageInterface {
        number: current?,
        endpoint_in: endpoint_in?,
        endpoint_out: endpoint_out?,
    })
}

const fn request(direction: u64, nr: u64, size: usize) -> u64 {
    (direction << 30) | ((size as u64) << 16) | ((b'U' as u64) << 8) | nr
}

const READ: u64 = 2;
const WRITE_READ: u64 = 3;

#[repr(C)]
struct BulkTransfer {
    endpoint: u32,
    length: u32,
    timeout_ms: u32,
    data: *mut libc::c_void,
}

#[repr(C)]
struct ControlTransfer {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
    timeout_ms: u32,
    data: *mut libc::c_void,
}

#[repr(C)]
struct InterfaceIoctl {
    interface: libc::c_int,
    code: libc::c_int,
    data: *mut libc::c_void,
}

const USBDEVFS_CONTROL: u64 = request(WRITE_READ, 0, size_of::<ControlTransfer>());
const USBDEVFS_BULK: u64 = request(WRITE_READ, 2, size_of::<BulkTransfer>());
const USBDEVFS_CLAIMINTERFACE: u64 = request(READ, 15, size_of::<u32>());
const USBDEVFS_RELEASEINTERFACE: u64 = request(READ, 16, size_of::<u32>());
const USBDEVFS_IOCTL: u64 = request(WRITE_READ, 18, size_of::<InterfaceIoctl>());
const USBDEVFS_CLEAR_HALT: u64 = request(READ, 21, size_of::<u32>());
const USBDEVFS_DISCONNECT: u64 = request(0, 22, 0);
const USBDEVFS_CONNECT: u64 = request(0, 23, 0);

/// Lance un transfert d'au plus `*limit` octets. Sans mémoire, le noyau refuse avant de toucher au bus : on
/// recommence plus petit, et la taille réduite vaut pour la suite.
fn with_shrinking_limit(
    limit: &mut usize,
    wanted: usize,
    mut transfer: impl FnMut(usize) -> io::Result<usize>,
) -> io::Result<usize> {
    loop {
        match transfer(wanted.min(*limit)) {
            Err(error) if error.raw_os_error() == Some(libc::ENOMEM) && *limit > TRANSFER_MIN => {
                *limit = (*limit / 2).max(TRANSFER_MIN);
                log::debug!(target: "bonk::usbfs", limit = *limit; "transfer size reduced");
            }
            result => return result,
        }
    }
}

fn bulk_error(error: io::Error) -> BulkError {
    match error.raw_os_error() {
        Some(libc::EPIPE) => BulkError::Stall,
        Some(libc::ENODEV | libc::ENOENT | libc::ESHUTDOWN) => BulkError::Disconnected,
        _ => BulkError::Other(error),
    }
}

/// Une interface de stockage réclamée. La lâcher rend la clé au pilote du noyau.
#[derive(Debug)]
pub struct UsbfsTransport {
    device: File,
    interface: StorageInterface,
    /// L'interface a été réclamée ici, et se rend ici. Sous Android, c'est l'application qui la tient.
    claimed_here: bool,
    transfer_limit: usize,
}

impl UsbfsTransport {
    /// Ouvre `/dev/bus/usb/BBB/DDD`, détache le pilote du noyau et réclame l'interface de stockage.
    ///
    /// # Errors
    /// Droits insuffisants, périphérique absent, ou pas d'interface de stockage « bulk-only ».
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::from_device(OpenOptions::new().read(true).write(true).open(path)?)
    }

    /// Même chose sur un descripteur déjà ouvert, comme celui que donne Android.
    ///
    /// # Errors
    /// Pas d'interface de stockage « bulk-only », ou réclamation refusée.
    pub(crate) fn from_device(mut device: File) -> io::Result<Self> {
        let mut descriptors = Vec::new();
        device.read_to_end(&mut descriptors)?;
        let interface = find_storage_interface(&descriptors).ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "no bulk-only storage interface")
        })?;
        let mut transport = Self {
            device,
            interface,
            claimed_here: true,
            transfer_limit: TRANSFER_START,
        };
        let mut detach = InterfaceIoctl {
            interface: libc::c_int::from(interface.number),
            code: USBDEVFS_DISCONNECT as libc::c_int,
            data: std::ptr::null_mut(),
        };
        // Sans pilote attaché, le système répond ENODATA : la voie est déjà libre.
        if let Err(error) = transport.ioctl(USBDEVFS_IOCTL, (&raw mut detach).cast())
            && error.raw_os_error() != Some(libc::ENODATA)
        {
            return Err(error);
        }
        let mut number = u32::from(interface.number);
        transport.ioctl(USBDEVFS_CLAIMINTERFACE, (&raw mut number).cast())?;
        Ok(transport)
    }

    /// Sur un descripteur dont l'interface est déjà réclamée : Android le fait par
    /// `claimInterface(interface, true)` et donne les descripteurs par `getRawDescriptors()`.
    ///
    /// # Errors
    /// Pas d'interface de stockage « bulk-only ».
    pub fn from_claimed(device: File, descriptors: &[u8]) -> io::Result<Self> {
        let interface = find_storage_interface(descriptors).ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "no bulk-only storage interface")
        })?;
        Ok(Self {
            device,
            interface,
            claimed_here: false,
            transfer_limit: TRANSFER_START,
        })
    }

    fn ioctl(&mut self, code: u64, argument: *mut libc::c_void) -> io::Result<usize> {
        // SAFETY: `argument` pointe sur la structure qu'attend `code`, vivante pendant l'appel, et les
        // tampons qu'elle désigne ont la longueur annoncée. Le cast suit le type de `ioctl` selon la libc.
        let done = unsafe { libc::ioctl(self.device.as_raw_fd(), code as _, argument) };
        usize::try_from(done).map_err(|_| io::Error::last_os_error())
    }

    fn bulk(&mut self, endpoint: u8, data: *mut u8, length: usize) -> Result<usize, BulkError> {
        let mut limit = self.transfer_limit;
        let result = with_shrinking_limit(&mut limit, length, |size| {
            let mut transfer = BulkTransfer {
                endpoint: u32::from(endpoint),
                // `size` ne dépasse jamais `TRANSFER_START`.
                length: size as u32,
                timeout_ms: TIMEOUT_MS,
                data: data.cast(),
            };
            self.ioctl(USBDEVFS_BULK, (&raw mut transfer).cast())
        });
        self.transfer_limit = limit;
        result.map_err(bulk_error)
    }
}

impl BulkTransport for UsbfsTransport {
    fn send(&mut self, data: &[u8]) -> Result<(), BulkError> {
        let endpoint = self.interface.endpoint_out;
        let mut sent = 0;
        while sent < data.len() {
            // Le système ne fait que lire ce tampon lors d'un envoi.
            let rest = data[sent..].as_ptr().cast_mut();
            match self.bulk(endpoint, rest, data.len() - sent)? {
                0 => {
                    return Err(BulkError::Other(io::Error::other(
                        "the drive accepted nothing",
                    )));
                }
                count => sent += count,
            }
        }
        Ok(())
    }

    fn receive(&mut self, buf: &mut [u8]) -> Result<usize, BulkError> {
        self.bulk(self.interface.endpoint_in, buf.as_mut_ptr(), buf.len())
    }

    fn clear_halt(&mut self, direction: Direction) -> Result<(), BulkError> {
        let mut endpoint = u32::from(match direction {
            Direction::In => self.interface.endpoint_in,
            Direction::Out => self.interface.endpoint_out,
        });
        self.ioctl(USBDEVFS_CLEAR_HALT, (&raw mut endpoint).cast())
            .map(|_| ())
            .map_err(bulk_error)
    }

    fn reset(&mut self) -> Result<(), BulkError> {
        let mut control = ControlTransfer {
            request_type: REQUEST_TYPE_CLASS_INTERFACE_OUT,
            request: REQUEST_MASS_STORAGE_RESET,
            value: 0,
            index: u16::from(self.interface.number),
            length: 0,
            timeout_ms: TIMEOUT_MS,
            data: std::ptr::null_mut(),
        };
        self.ioctl(USBDEVFS_CONTROL, (&raw mut control).cast())
            .map_err(bulk_error)?;
        self.clear_halt(Direction::In)?;
        self.clear_halt(Direction::Out)
    }
}

impl Drop for UsbfsTransport {
    fn drop(&mut self) {
        if !self.claimed_here {
            return;
        }
        let mut number = u32::from(self.interface.number);
        let released = self.ioctl(USBDEVFS_RELEASEINTERFACE, (&raw mut number).cast());
        let mut attach = InterfaceIoctl {
            interface: libc::c_int::from(self.interface.number),
            code: USBDEVFS_CONNECT as libc::c_int,
            data: std::ptr::null_mut(),
        };
        let attached = self.ioctl(USBDEVFS_IOCTL, (&raw mut attach).cast());
        if released.is_err() || attached.is_err() {
            log::warn!(target: "bonk::usbfs", "the drive was not handed back to the kernel driver");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clé réelle type : périphérique, configuration, interface de stockage, deux points de terminaison.
    fn stockage() -> Vec<u8> {
        let mut bytes = vec![18, 1];
        bytes.extend([0; 16]);
        bytes.extend([9, 2, 32, 0, 1, 1, 0, 0x80, 50]);
        bytes.extend([
            9,
            4,
            3,
            0,
            2,
            CLASS_STORAGE,
            SUBCLASS_SCSI,
            PROTOCOL_BULK_ONLY,
            0,
        ]);
        bytes.extend([7, 5, 0x81, 2, 0, 2, 0]);
        bytes.extend([7, 5, 0x02, 2, 0, 2, 0]);
        bytes
    }

    #[test]
    fn trouve_l_interface_et_ses_deux_points_de_terminaison() {
        assert_eq!(
            find_storage_interface(&stockage()),
            Some(StorageInterface {
                number: 3,
                endpoint_in: 0x81,
                endpoint_out: 0x02
            })
        );
    }

    #[test]
    fn ignore_une_autre_classe_et_prend_le_stockage_qui_suit() {
        let mut bytes = vec![9, 4, 0, 0, 1, 3, 1, 1, 0];
        bytes.extend([7, 5, 0x83, 3, 8, 0, 10]);
        bytes.extend(stockage());
        assert_eq!(find_storage_interface(&bytes).map(|i| i.number), Some(3));
    }

    #[test]
    fn refuse_uas_un_point_manquant_et_des_descripteurs_tronques() {
        let uas = [9, 4, 0, 0, 2, CLASS_STORAGE, SUBCLASS_SCSI, 0x62, 0];
        assert_eq!(find_storage_interface(&uas), None);
        let complet = stockage();
        assert_eq!(find_storage_interface(&complet[..complet.len() - 7]), None);
        assert_eq!(find_storage_interface(&complet[..complet.len() - 3]), None);
        assert_eq!(find_storage_interface(&[0, 0]), None);
        assert_eq!(find_storage_interface(&[]), None);
    }

    /// Valeurs de `linux/usbdevice_fs.h` sur une architecture 64 bits.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn numeros_d_appel_du_noyau() {
        assert_eq!(USBDEVFS_CONTROL, 0xC018_5500);
        assert_eq!(USBDEVFS_BULK, 0xC018_5502);
        assert_eq!(USBDEVFS_CLAIMINTERFACE, 0x8004_550F);
        assert_eq!(USBDEVFS_RELEASEINTERFACE, 0x8004_5510);
        assert_eq!(USBDEVFS_IOCTL, 0xC010_5512);
        assert_eq!(USBDEVFS_CLEAR_HALT, 0x8004_5515);
        assert_eq!(USBDEVFS_DISCONNECT, 0x5516);
        assert_eq!(USBDEVFS_CONNECT, 0x5517);
    }

    #[test]
    fn sans_memoire_le_transfert_recommence_plus_petit_et_le_reste() -> io::Result<()> {
        let mut limit = TRANSFER_START;
        let mut sizes = Vec::new();
        let done = with_shrinking_limit(&mut limit, 1 << 20, |size| {
            sizes.push(size);
            if size > 64 << 10 {
                Err(io::Error::from_raw_os_error(libc::ENOMEM))
            } else {
                Ok(size)
            }
        })?;
        assert_eq!(sizes, [256 << 10, 128 << 10, 64 << 10]);
        assert_eq!((done, limit), (64 << 10, 64 << 10));
        Ok(())
    }

    #[test]
    fn au_plus_petit_le_manque_de_memoire_remonte() {
        let mut limit = TRANSFER_MIN;
        let result = with_shrinking_limit(&mut limit, 1 << 20, |_| {
            Err(io::Error::from_raw_os_error(libc::ENOMEM))
        });
        assert!(result.is_err());
    }

    #[test]
    fn une_autre_erreur_ne_reduit_rien() {
        let mut limit = TRANSFER_START;
        let result = with_shrinking_limit(&mut limit, 1 << 20, |_| {
            Err(io::Error::from_raw_os_error(libc::EPIPE))
        });
        assert!(result.is_err());
        assert_eq!(limit, TRANSFER_START);
    }

    #[test]
    fn les_erreurs_du_systeme_gardent_leur_sens() {
        let erreur = |code| bulk_error(io::Error::from_raw_os_error(code));
        assert!(matches!(erreur(libc::EPIPE), BulkError::Stall));
        assert!(matches!(erreur(libc::ENODEV), BulkError::Disconnected));
        assert!(matches!(erreur(libc::EIO), BulkError::Other(_)));
    }
}
