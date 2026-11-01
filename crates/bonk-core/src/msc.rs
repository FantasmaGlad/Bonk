//! Stockage de masse USB depuis l'espace utilisateur : transport « bulk-only » et commandes SCSI.
//!
//! Sur Android sans root, l'API ne donne que des transferts USB bruts : il faut parler le protocole de la
//! clé soi-même. Cette couche est pure : elle ne connaît qu'un [`BulkTransport`], fourni par chaque système,
//! et se teste contre un faux périphérique en mémoire.

use std::cell::RefCell;
use std::fmt;
use std::io;

use crate::block::{BlockDevice, BlockError, check_access, check_geometry};

const CBW_SIGNATURE: u32 = 0x4342_5355;
const CSW_SIGNATURE: u32 = 0x5342_5355;
const CBW_LEN: usize = 31;
const CSW_LEN: usize = 13;
/// Plafond d'une commande : 1 Mio, que les clés acceptent et qui laisse l'annulation réagir vite.
const MAX_TRANSFER: usize = 1 << 20;
/// Une clé qu'on vient de brancher répond « pas prête » un moment.
const READY_ATTEMPTS: u32 = 5;

/// Sens d'un transfert, vu de l'hôte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

/// Échec d'un transfert en bloc.
#[derive(Debug)]
pub enum BulkError {
    /// Le périphérique a arrêté le point de terminaison.
    Stall,
    /// Le périphérique n'est plus là.
    Disconnected,
    Other(io::Error),
}

/// Transferts en bloc vers une interface de stockage déjà réclamée.
pub trait BulkTransport {
    /// Envoie tout `data`.
    ///
    /// # Errors
    /// Arrêt du point de terminaison, débranchement ou erreur du système.
    fn send(&mut self, data: &[u8]) -> Result<(), BulkError>;
    /// Reçoit au plus `buf.len()` octets et rend le nombre reçu.
    ///
    /// # Errors
    /// Arrêt du point de terminaison, débranchement ou erreur du système.
    fn receive(&mut self, buf: &mut [u8]) -> Result<usize, BulkError>;
    /// Efface l'arrêt d'un point de terminaison.
    ///
    /// # Errors
    /// Débranchement ou erreur du système.
    fn clear_halt(&mut self, direction: Direction) -> Result<(), BulkError>;
    /// Réinitialisation du stockage de masse : requête de classe, puis effacement des deux arrêts.
    ///
    /// # Errors
    /// Débranchement ou erreur du système.
    fn reset(&mut self) -> Result<(), BulkError>;
}

const SENSE_ILLEGAL_REQUEST: u8 = 5;

/// Ce que dit la clé d'une commande refusée : clé de sens, code et qualificatif SCSI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sense {
    pub key: u8,
    pub code: u8,
    pub qualifier: u8,
}

/// Échec d'une commande.
#[derive(Debug)]
pub enum MscError {
    Bulk(BulkError),
    /// Enveloppe de statut mal formée, ou qui ne répond pas à cette commande.
    Protocol(&'static str),
    /// Erreur de phase : l'échange est perdu, le périphérique a été réinitialisé.
    Phase,
    /// La commande a échoué ; le sens est absent s'il n'a pas pu être lu.
    Check(Option<Sense>),
    /// Moins d'octets transférés que demandé.
    Short {
        expected: usize,
        done: usize,
    },
}

impl fmt::Display for MscError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bulk(BulkError::Stall) => f.write_str("USB endpoint stalled"),
            Self::Bulk(BulkError::Disconnected) => f.write_str("USB device disconnected"),
            Self::Bulk(BulkError::Other(err)) => write!(f, "USB transfer failed: {err}"),
            Self::Protocol(what) => write!(f, "USB storage protocol error: {what}"),
            Self::Phase => f.write_str("USB storage phase error"),
            Self::Check(Some(sense)) => write!(
                f,
                "SCSI command failed: sense {:02x}/{:02x}/{:02x}",
                sense.key, sense.code, sense.qualifier
            ),
            Self::Check(None) => f.write_str("SCSI command failed"),
            Self::Short { expected, done } => {
                write!(f, "short USB transfer: {done} bytes of {expected}")
            }
        }
    }
}

impl std::error::Error for MscError {}

impl From<BulkError> for MscError {
    fn from(error: BulkError) -> Self {
        Self::Bulk(error)
    }
}

impl From<MscError> for BlockError {
    fn from(error: MscError) -> Self {
        let kind = match &error {
            MscError::Bulk(BulkError::Disconnected) => io::ErrorKind::NotConnected,
            _ => io::ErrorKind::Other,
        };
        Self::Io(io::Error::new(kind, error))
    }
}

/// Phase de données d'une commande.
enum Data<'a> {
    None,
    In(&'a mut [u8]),
    Out(&'a [u8]),
}

/// Résultat de `INQUIRY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inquiry {
    pub vendor: String,
    pub product: String,
}

/// Session sur une interface de stockage.
pub struct MassStorage<T> {
    transport: T,
    tag: u32,
}

impl<T: BulkTransport> MassStorage<T> {
    pub fn new(transport: T) -> Self {
        Self { transport, tag: 0 }
    }

    /// Envoie une commande et rend les octets transférés. `Check` ne porte pas encore le sens.
    fn execute(&mut self, cdb: &[u8], data: Data<'_>) -> Result<usize, MscError> {
        self.tag = self.tag.wrapping_add(1);
        let tag = self.tag;
        let (length, flags) = match &data {
            Data::None => (0, 0),
            Data::In(buf) => (buf.len(), 0x80),
            Data::Out(buf) => (buf.len(), 0),
        };
        let mut cbw = [0u8; CBW_LEN];
        cbw[0..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
        cbw[4..8].copy_from_slice(&tag.to_le_bytes());
        cbw[8..12].copy_from_slice(&u32::try_from(length).unwrap_or(u32::MAX).to_le_bytes());
        cbw[12] = flags;
        cbw[14] = u8::try_from(cdb.len()).unwrap_or(16);
        cbw[15..15 + cdb.len()].copy_from_slice(cdb);
        if let Err(error) = self.transport.send(&cbw) {
            if matches!(error, BulkError::Stall) {
                self.transport.reset()?;
            }
            return Err(error.into());
        }

        let mut done = 0;
        match data {
            Data::None => {}
            Data::In(buf) => {
                while done < buf.len() {
                    match self.transport.receive(&mut buf[done..]) {
                        Ok(0) => break,
                        Ok(read) => done += read,
                        Err(BulkError::Stall) => {
                            self.transport.clear_halt(Direction::In)?;
                            break;
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
            }
            Data::Out(buf) => match self.transport.send(buf) {
                Ok(()) => done = buf.len(),
                Err(BulkError::Stall) => self.transport.clear_halt(Direction::Out)?,
                Err(error) => return Err(error.into()),
            },
        }

        let csw = self.read_status()?;
        if u32::from_le_bytes([csw[0], csw[1], csw[2], csw[3]]) != CSW_SIGNATURE {
            self.transport.reset()?;
            return Err(MscError::Protocol("bad status signature"));
        }
        if u32::from_le_bytes([csw[4], csw[5], csw[6], csw[7]]) != tag {
            self.transport.reset()?;
            return Err(MscError::Protocol("status of another command"));
        }
        match csw[12] {
            0 => {}
            1 => return Err(MscError::Check(None)),
            2 => {
                self.transport.reset()?;
                return Err(MscError::Phase);
            }
            _ => {
                self.transport.reset()?;
                return Err(MscError::Protocol("unknown status"));
            }
        }
        // Le résidu de la clé compte autant que ce qu'on a compté : une clé peut combler avec des zéros.
        let residue = u32::from_le_bytes([csw[8], csw[9], csw[10], csw[11]]) as usize;
        Ok(done.min(length.saturating_sub(residue)))
    }

    /// Un arrêt pendant le statut se lève une fois (spécification « bulk-only ») ; au second, réinitialise.
    fn read_status(&mut self) -> Result<[u8; CSW_LEN], MscError> {
        let mut csw = [0u8; CSW_LEN];
        for attempt in 0..2 {
            match self.transport.receive(&mut csw) {
                Ok(CSW_LEN) => return Ok(csw),
                Ok(_) => {
                    self.transport.reset()?;
                    return Err(MscError::Protocol("short status"));
                }
                Err(BulkError::Stall) if attempt == 0 => {
                    self.transport.clear_halt(Direction::In)?;
                }
                Err(BulkError::Stall) => {
                    self.transport.reset()?;
                    return Err(MscError::Bulk(BulkError::Stall));
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(MscError::Protocol("no status"))
    }

    /// Exécute une commande ; en cas d'échec, lit le sens pour le rapporter.
    fn command(&mut self, cdb: &[u8], data: Data<'_>) -> Result<usize, MscError> {
        match self.execute(cdb, data) {
            Err(MscError::Check(_)) => Err(MscError::Check(self.request_sense().ok())),
            other => other,
        }
    }

    fn request_sense(&mut self) -> Result<Sense, MscError> {
        let mut reply = [0u8; 18];
        self.execute(&[0x03, 0, 0, 0, 18, 0], Data::In(&mut reply))?;
        Ok(Sense {
            key: reply[2] & 0x0F,
            code: reply[12],
            qualifier: reply[13],
        })
    }

    /// `INQUIRY` : fabricant et produit.
    ///
    /// # Errors
    /// Échec de la commande.
    pub fn inquiry(&mut self) -> Result<Inquiry, MscError> {
        let mut reply = [0u8; 36];
        let done = self.command(&[0x12, 0, 0, 0, 36, 0], Data::In(&mut reply))?;
        if done < 32 {
            return Err(MscError::Short { expected: 36, done });
        }
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_owned();
        Ok(Inquiry {
            vendor: text(&reply[8..16]),
            product: text(&reply[16..32]),
        })
    }

    /// Attend que la clé soit prête : une clé tout juste branchée signale d'abord son changement.
    ///
    /// # Errors
    /// Toujours pas prête après quelques essais.
    pub fn wait_ready(&mut self) -> Result<(), MscError> {
        let mut last = MscError::Check(None);
        for _ in 0..READY_ATTEMPTS {
            match self.command(&[0, 0, 0, 0, 0, 0], Data::None) {
                Ok(_) => return Ok(()),
                Err(error @ MscError::Check(_)) => last = error,
                Err(error) => return Err(error),
            }
        }
        Err(last)
    }

    /// Capacité : nombre de blocs et taille d'un bloc. `READ CAPACITY(16)` seulement si (10) ne suffit pas.
    ///
    /// # Errors
    /// Échec de la commande, ou réponse trop courte.
    pub fn capacity(&mut self) -> Result<(u64, u32), MscError> {
        let mut short = [0u8; 8];
        let done = self.command(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], Data::In(&mut short))?;
        if done < 8 {
            return Err(MscError::Short { expected: 8, done });
        }
        let last = u32::from_be_bytes([short[0], short[1], short[2], short[3]]);
        let block = u32::from_be_bytes([short[4], short[5], short[6], short[7]]);
        if last != u32::MAX {
            return Ok((u64::from(last) + 1, block));
        }
        let mut long = [0u8; 32];
        let mut cdb = [0u8; 16];
        cdb[0] = 0x9E;
        cdb[1] = 0x10;
        cdb[13] = 32;
        let done = self.command(&cdb, Data::In(&mut long))?;
        if done < 12 {
            return Err(MscError::Short { expected: 32, done });
        }
        let mut last = [0u8; 8];
        last.copy_from_slice(&long[..8]);
        Ok((
            u64::from_be_bytes(last) + 1,
            u32::from_be_bytes([long[8], long[9], long[10], long[11]]),
        ))
    }

    /// Commande de lecture ou d'écriture : la forme 10 tant qu'elle suffit, sinon la forme 16.
    fn rw_cdb(opcode_10: u8, opcode_16: u8, lba: u64, blocks: u32) -> Vec<u8> {
        if let (Ok(short_lba), Ok(short_blocks)) = (u32::try_from(lba), u16::try_from(blocks)) {
            let mut cdb = vec![0u8; 10];
            cdb[0] = opcode_10;
            cdb[2..6].copy_from_slice(&short_lba.to_be_bytes());
            cdb[7..9].copy_from_slice(&short_blocks.to_be_bytes());
            cdb
        } else {
            let mut cdb = vec![0u8; 16];
            cdb[0] = opcode_16;
            cdb[2..10].copy_from_slice(&lba.to_be_bytes());
            cdb[10..14].copy_from_slice(&blocks.to_be_bytes());
            cdb
        }
    }

    fn read_blocks(&mut self, lba: u64, buf: &mut [u8], block: u32) -> Result<(), MscError> {
        let blocks = u32::try_from(buf.len() / block as usize).unwrap_or(u32::MAX);
        let cdb = Self::rw_cdb(0x28, 0x88, lba, blocks);
        let expected = buf.len();
        let done = self.command(&cdb, Data::In(buf))?;
        if done == expected {
            Ok(())
        } else {
            Err(MscError::Short { expected, done })
        }
    }

    fn write_blocks(&mut self, lba: u64, data: &[u8], block: u32) -> Result<(), MscError> {
        let blocks = u32::try_from(data.len() / block as usize).unwrap_or(u32::MAX);
        let cdb = Self::rw_cdb(0x2A, 0x8A, lba, blocks);
        let done = self.command(&cdb, Data::Out(data))?;
        if done == data.len() {
            Ok(())
        } else {
            Err(MscError::Short {
                expected: data.len(),
                done,
            })
        }
    }

    fn synchronize(&mut self) -> Result<(), MscError> {
        match self.command(&[0x35, 0, 0, 0, 0, 0, 0, 0, 0, 0], Data::None) {
            // Des clés sans cache d'écriture rejettent cette commande comme invalide : il n'y a rien à
            // vider, et la relecture après écriture reste le vrai contrôle. Tout autre refus est une erreur.
            Err(MscError::Check(Some(Sense {
                key: SENSE_ILLEGAL_REQUEST,
                ..
            }))) => Ok(()),
            result => result.map(|_| ()),
        }
    }
}

/// Une clé USB vue comme [`BlockDevice`].
pub struct UsbDisk<T> {
    session: RefCell<MassStorage<T>>,
    size: u64,
    block_size: u32,
}

impl<T: BulkTransport> UsbDisk<T> {
    /// Attend la clé et lit sa capacité.
    ///
    /// # Errors
    /// Clé qui ne répond pas, ou géométrie invalide.
    pub fn open(transport: T) -> Result<Self, BlockError> {
        let mut session = MassStorage::new(transport);
        session.wait_ready()?;
        let (blocks, block_size) = session.capacity()?;
        let size =
            blocks
                .checked_mul(u64::from(block_size))
                .ok_or(BlockError::InvalidGeometry {
                    size: u64::MAX,
                    block_size,
                })?;
        check_geometry(size, block_size)?;
        Ok(Self {
            session: RefCell::new(session),
            size,
            block_size,
        })
    }
}

impl<T: BulkTransport> BlockDevice for UsbDisk<T> {
    fn size(&self) -> u64 {
        self.size
    }

    fn block_size(&self) -> u32 {
        self.block_size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        check_access(self.size, self.block_size, offset, buf.len())?;
        let block = u64::from(self.block_size);
        let chunk = MAX_TRANSFER - MAX_TRANSFER % self.block_size as usize;
        let mut session = self.session.borrow_mut();
        let mut lba = offset / block;
        for part in buf.chunks_mut(chunk) {
            session.read_blocks(lba, part, self.block_size)?;
            lba += part.len() as u64 / block;
        }
        Ok(())
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        check_access(self.size, self.block_size, offset, data.len())?;
        let block = u64::from(self.block_size);
        let chunk = MAX_TRANSFER - MAX_TRANSFER % self.block_size as usize;
        let session = self.session.get_mut();
        let mut lba = offset / block;
        for part in data.chunks(chunk) {
            session.write_blocks(lba, part, self.block_size)?;
            lba += part.len() as u64 / block;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.session.get_mut().synchronize()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: usize = 512;

    /// Panne à provoquer sur la prochaine commande dont l'opcode correspond.
    #[derive(Clone, Copy, PartialEq)]
    enum Fault {
        None,
        StallDataIn,
        StallDataOut,
        StallStatusOnce,
        Phase,
        Check,
        BadTag,
        BadSignature,
        Gone,
    }

    /// Clé USB en mémoire qui suit le protocole.
    struct Fake {
        storage: Vec<u8>,
        huge: bool,
        fault: (u8, Fault),
        pending_in: Vec<u8>,
        pending_csw: Option<[u8; CSW_LEN]>,
        expect_out: Option<(u64, usize)>,
        out: Vec<u8>,
        tag: u32,
        status: u8,
        residue: u32,
        not_ready: u32,
        resets: u32,
        clears: Vec<Direction>,
        stall_status: bool,
        stall_in: bool,
        stall_out: bool,
        commands: Vec<u8>,
        max_blocks: usize,
        sense_key: u8,
    }

    impl Fake {
        fn new(blocks: usize) -> Self {
            Self {
                sense_key: 6,
                storage: vec![0; blocks * BLOCK],
                huge: false,
                fault: (0, Fault::None),
                pending_in: Vec::new(),
                pending_csw: None,
                expect_out: None,
                out: Vec::new(),
                tag: 0,
                status: 0,
                residue: 0,
                not_ready: 0,
                resets: 0,
                clears: Vec::new(),
                stall_status: false,
                stall_in: false,
                stall_out: false,
                commands: Vec::new(),
                max_blocks: 0,
            }
        }

        fn csw(&self) -> [u8; CSW_LEN] {
            let mut csw = [0u8; CSW_LEN];
            csw[0..4].copy_from_slice(&CSW_SIGNATURE.to_le_bytes());
            csw[4..8].copy_from_slice(&self.tag.to_le_bytes());
            csw[8..12].copy_from_slice(&self.residue.to_le_bytes());
            csw[12] = self.status;
            csw
        }

        fn handle_cbw(&mut self, cbw: &[u8]) {
            assert_eq!(cbw.len(), CBW_LEN);
            assert_eq!(
                u32::from_le_bytes([cbw[0], cbw[1], cbw[2], cbw[3]]),
                CBW_SIGNATURE
            );
            self.tag = u32::from_le_bytes([cbw[4], cbw[5], cbw[6], cbw[7]]);
            let length = u32::from_le_bytes([cbw[8], cbw[9], cbw[10], cbw[11]]) as usize;
            let cdb = &cbw[15..15 + cbw[14] as usize];
            self.commands.push(cdb[0]);
            self.status = 0;
            self.residue = 0;
            self.pending_in.clear();
            let fault = if self.fault.0 == cdb[0] {
                std::mem::replace(&mut self.fault.1, Fault::None)
            } else {
                Fault::None
            };
            match fault {
                Fault::StallDataIn => {
                    self.stall_in = true;
                    self.status = 1;
                    self.residue = u32::try_from(length).unwrap_or(0);
                }
                Fault::StallDataOut => {
                    self.stall_out = true;
                    self.status = 1;
                }
                Fault::StallStatusOnce => self.stall_status = true,
                // Une commande refusée n'a pas de données à rendre : la clé arrête le point de terminaison.
                Fault::Phase => {
                    self.status = 2;
                    self.stall_in = length > 0;
                }
                Fault::Check => {
                    self.status = 1;
                    self.stall_in = length > 0;
                }
                Fault::BadTag => self.tag = self.tag.wrapping_add(7),
                Fault::BadSignature => {}
                Fault::Gone => self.fault = (0xFE, Fault::Gone),
                Fault::None => {}
            }
            let bad_signature = fault == Fault::BadSignature;
            if self.status == 0 {
                match cdb[0] {
                    0x00 => {
                        if self.not_ready > 0 {
                            self.not_ready -= 1;
                            self.status = 1;
                        }
                    }
                    0x03 => {
                        let mut sense = vec![0u8; 18];
                        sense[2] = self.sense_key;
                        sense[12] = 0x28;
                        self.pending_in = sense;
                    }
                    0x12 => {
                        let mut reply = vec![b' '; 36];
                        reply[8..15].copy_from_slice(b"General");
                        reply[16..21].copy_from_slice(b"UDisk");
                        self.pending_in = reply;
                    }
                    0x25 => {
                        let last = if self.huge {
                            u32::MAX
                        } else {
                            u32::try_from(self.storage.len() / BLOCK - 1).unwrap_or(0)
                        };
                        let mut reply = last.to_be_bytes().to_vec();
                        reply.extend_from_slice(&(BLOCK as u32).to_be_bytes());
                        self.pending_in = reply;
                    }
                    0x9E => {
                        let mut reply = vec![0u8; 32];
                        reply[..8].copy_from_slice(&(1u64 << 33).to_be_bytes());
                        reply[8..12].copy_from_slice(&(BLOCK as u32).to_be_bytes());
                        self.pending_in = reply;
                    }
                    0x28 | 0x88 | 0x2A | 0x8A => {
                        let (lba, blocks) = if cdb.len() == 10 {
                            (
                                u64::from(u32::from_be_bytes([cdb[2], cdb[3], cdb[4], cdb[5]])),
                                usize::from(u16::from_be_bytes([cdb[7], cdb[8]])),
                            )
                        } else {
                            let mut lba = [0u8; 8];
                            lba.copy_from_slice(&cdb[2..10]);
                            (
                                u64::from_be_bytes(lba),
                                u32::from_be_bytes([cdb[10], cdb[11], cdb[12], cdb[13]]) as usize,
                            )
                        };
                        self.max_blocks = self.max_blocks.max(blocks);
                        let start = lba as usize * BLOCK;
                        if matches!(cdb[0], 0x28 | 0x88) {
                            self.pending_in = self.storage[start..start + blocks * BLOCK].to_vec();
                        } else {
                            self.expect_out = Some((lba, blocks * BLOCK));
                            self.out.clear();
                        }
                    }
                    _ => {}
                }
            }
            let mut csw = self.csw();
            if bad_signature {
                csw[0] = 0;
            }
            self.pending_csw = Some(csw);
        }
    }

    impl BulkTransport for &mut Fake {
        fn send(&mut self, data: &[u8]) -> Result<(), BulkError> {
            if self.fault == (0xFE, Fault::Gone) {
                return Err(BulkError::Disconnected);
            }
            if self.stall_out {
                self.stall_out = false;
                return Err(BulkError::Stall);
            }
            if let Some((lba, length)) = self.expect_out {
                self.out.extend_from_slice(data);
                if self.out.len() >= length {
                    let start = lba as usize * BLOCK;
                    self.storage[start..start + length].copy_from_slice(&self.out);
                    self.expect_out = None;
                }
                return Ok(());
            }
            if data.len() == CBW_LEN && data[..4] == CBW_SIGNATURE.to_le_bytes() {
                self.handle_cbw(data);
            }
            Ok(())
        }

        fn receive(&mut self, buf: &mut [u8]) -> Result<usize, BulkError> {
            if self.fault == (0xFE, Fault::Gone) {
                return Err(BulkError::Disconnected);
            }
            if self.stall_in {
                self.stall_in = false;
                return Err(BulkError::Stall);
            }
            if !self.pending_in.is_empty() {
                let count = buf.len().min(self.pending_in.len());
                buf[..count].copy_from_slice(&self.pending_in[..count]);
                self.pending_in.drain(..count);
                return Ok(count);
            }
            if self.stall_status {
                self.stall_status = false;
                return Err(BulkError::Stall);
            }
            let csw = self.pending_csw.take().ok_or(BulkError::Stall)?;
            buf[..CSW_LEN].copy_from_slice(&csw);
            Ok(CSW_LEN)
        }

        fn clear_halt(&mut self, direction: Direction) -> Result<(), BulkError> {
            self.clears.push(direction);
            Ok(())
        }

        fn reset(&mut self) -> Result<(), BulkError> {
            self.resets += 1;
            Ok(())
        }
    }

    fn random(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn ecriture_puis_lecture_a_travers_le_protocole() -> Result<(), BlockError> {
        let mut fake = Fake::new(8192);
        let mut disk = UsbDisk::open(&mut fake)?;
        assert_eq!(disk.size(), 8192 * BLOCK as u64);
        let data = random(3 << 20);
        disk.write_at(4096, &data)?;
        disk.flush()?;
        let mut back = vec![0u8; data.len()];
        disk.read_at(4096, &mut back)?;
        assert_eq!(back, data);
        assert_eq!(&fake.storage[4096..4096 + data.len()], &data[..]);
        assert!(fake.commands.contains(&0x35));
        // Découpé : aucune commande au-delà du plafond.
        assert!(fake.max_blocks <= MAX_TRANSFER / BLOCK);
        Ok(())
    }

    #[test]
    fn identification_et_grande_capacite() -> Result<(), MscError> {
        let mut fake = Fake::new(16);
        fake.huge = true;
        let mut session = MassStorage::new(&mut fake);
        assert_eq!(
            session.inquiry()?,
            Inquiry {
                vendor: "General".to_owned(),
                product: "UDisk".to_owned()
            }
        );
        assert_eq!(session.capacity()?, ((1 << 33) + 1, BLOCK as u32));
        Ok(())
    }

    #[test]
    fn au_dela_de_deux_teraoctets_la_forme_16_sert() {
        let cdb = MassStorage::<&mut Fake>::rw_cdb(0x28, 0x88, 1 << 33, 8);
        assert_eq!((cdb.len(), cdb[0]), (16, 0x88));
        let cdb = MassStorage::<&mut Fake>::rw_cdb(0x28, 0x88, 5, 8);
        assert_eq!((cdb.len(), cdb[0]), (10, 0x28));
    }

    #[test]
    fn une_cle_qui_se_reveille_devient_prete() -> Result<(), MscError> {
        let mut fake = Fake::new(16);
        fake.not_ready = 2;
        MassStorage::new(&mut fake).wait_ready()?;
        assert_eq!(fake.commands.iter().filter(|c| **c == 0).count(), 3);
        // Le sens est lu après chaque refus.
        assert_eq!(fake.commands.iter().filter(|c| **c == 3).count(), 2);

        let mut jamais = Fake::new(16);
        jamais.not_ready = 100;
        assert!(matches!(
            MassStorage::new(&mut jamais).wait_ready(),
            Err(MscError::Check(Some(Sense {
                key: 6,
                code: 0x28,
                ..
            })))
        ));
        Ok(())
    }

    #[test]
    fn une_cle_sans_cache_peut_refuser_de_le_vider() -> Result<(), BlockError> {
        let mut fake = Fake::new(16);
        fake.fault = (0x35, Fault::Check);
        fake.sense_key = SENSE_ILLEGAL_REQUEST;
        UsbDisk::open(&mut fake)?.flush()
    }

    #[test]
    fn tout_autre_refus_de_vider_le_cache_reste_une_erreur() -> Result<(), BlockError> {
        let mut fake = Fake::new(16);
        fake.fault = (0x35, Fault::Check);
        assert!(UsbDisk::open(&mut fake)?.flush().is_err());
        Ok(())
    }

    #[test]
    fn un_echec_rapporte_le_sens() {
        let mut fake = Fake::new(16);
        fake.fault = (0x12, Fault::Check);
        assert!(matches!(
            MassStorage::new(&mut fake).inquiry(),
            Err(MscError::Check(Some(Sense { key: 6, .. })))
        ));
    }

    #[test]
    fn un_arret_en_lecture_est_leve_et_rapporte() {
        let mut fake = Fake::new(16);
        fake.fault = (0x28, Fault::StallDataIn);
        let mut session = MassStorage::new(&mut fake);
        let mut buf = vec![0u8; BLOCK];
        assert!(matches!(
            session.read_blocks(0, &mut buf, BLOCK as u32),
            Err(MscError::Check(_))
        ));
        assert_eq!(fake.clears, [Direction::In]);
    }

    #[test]
    fn un_arret_en_ecriture_n_ecrit_rien_et_le_dit() {
        let mut fake = Fake::new(16);
        fake.fault = (0x2A, Fault::StallDataOut);
        assert!(matches!(
            MassStorage::new(&mut fake).write_blocks(0, &[9u8; BLOCK], BLOCK as u32),
            Err(MscError::Check(_))
        ));
        assert_eq!(fake.clears, [Direction::Out]);
        assert!(fake.storage.iter().all(|b| *b == 0));
    }

    #[test]
    fn un_arret_du_statut_se_rattrape_une_fois() -> Result<(), MscError> {
        let mut fake = Fake::new(16);
        fake.fault = (0x12, Fault::StallStatusOnce);
        MassStorage::new(&mut fake).inquiry()?;
        assert_eq!(fake.clears, [Direction::In]);
        assert_eq!(fake.resets, 0);
        Ok(())
    }

    #[test]
    fn erreur_de_phase_et_statut_faux_reinitialisent() {
        for fault in [Fault::Phase, Fault::BadTag, Fault::BadSignature] {
            let mut fake = Fake::new(16);
            fake.fault = (0x12, fault);
            let result = MassStorage::new(&mut fake).inquiry();
            assert!(
                matches!(result, Err(MscError::Phase | MscError::Protocol(_))),
                "{result:?}"
            );
            assert_eq!(fake.resets, 1);
        }
    }

    #[test]
    fn une_cle_debranchee_devient_une_erreur_de_bloc_reconnaissable() -> Result<(), BlockError> {
        let mut fake = Fake::new(16);
        let disk = UsbDisk::open(&mut fake)?;
        disk.session.borrow_mut().transport.fault = (0xFE, Fault::Gone);
        let mut buf = vec![0u8; BLOCK];
        match disk.read_at(0, &mut buf) {
            Err(BlockError::Io(error)) => assert_eq!(error.kind(), io::ErrorKind::NotConnected),
            other => panic!("{other:?}"),
        }
        Ok(())
    }

    #[test]
    fn un_acces_hors_bornes_ne_part_pas_sur_le_bus() -> Result<(), BlockError> {
        let mut fake = Fake::new(16);
        let mut disk = UsbDisk::open(&mut fake)?;
        let avant = disk.session.borrow().transport.commands.len();
        assert!(disk.write_at(16 * BLOCK as u64, &[0; BLOCK]).is_err());
        assert!(disk.write_at(1, &[0; BLOCK]).is_err());
        assert_eq!(disk.session.borrow().transport.commands.len(), avant);
        Ok(())
    }
}
