use std::io;

use super::*;
use crate::block::{BlockDevice, BlockError};

const BLOCK: usize = 512;

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
    KeepsPart,
}

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
    sense_code: u8,
    /// Bytes returned to `INQUIRY`: fewer than 36 ends with a short packet.
    inquiry_len: usize,
}

impl Fake {
    fn new(blocks: usize) -> Self {
        Self {
            sense_key: 6,
            sense_code: 0x28,
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
            inquiry_len: 36,
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

    fn apply(&mut self, fault: Fault, length: usize) {
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
            // A refused command has no data: the device stalls the endpoint.
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
            Fault::KeepsPart => self.residue = u32::try_from(BLOCK).unwrap_or(0),
            Fault::None => {}
        }
    }

    fn answer(&mut self, cdb: &[u8]) {
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
                sense[12] = self.sense_code;
                self.pending_in = sense;
            }
            0x12 => {
                let mut reply = vec![b' '; 36];
                reply[8..15].copy_from_slice(b"General");
                reply[16..21].copy_from_slice(b"UDisk");
                reply.truncate(self.inquiry_len);
                self.residue = u32::try_from(36 - self.inquiry_len).unwrap_or(0);
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
        self.apply(fault, length);
        let bad_signature = fault == Fault::BadSignature;
        if self.status == 0 {
            self.answer(cdb);
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
        // The status does not fit a smaller data buffer: the kernel returns EOVERFLOW.
        if buf.len() < CSW_LEN {
            return Err(BulkError::Other(io::Error::other(
                "babble: status in data buffer",
            )));
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
fn une_reponse_courte_n_avale_pas_le_statut() -> Result<(), MscError> {
    for len in [34, 32] {
        let mut fake = Fake::new(64);
        fake.inquiry_len = len;
        let mut msc = MassStorage::new(&mut fake);
        let inquiry = msc.inquiry()?;
        assert_eq!(inquiry.vendor, "General", "{len} octets");
        assert_eq!(inquiry.product, "UDisk", "{len} octets");
        assert_eq!(msc.capacity()?, (64, BLOCK as u32), "{len} octets");
    }
    Ok(())
}

#[test]
fn une_ecriture_gardee_en_partie_est_un_echec() -> Result<(), BlockError> {
    let mut fake = Fake::new(64);
    fake.fault = (0x2A, Fault::KeepsPart);
    let mut disk = UsbDisk::open(&mut fake)?;
    let erreur = disk.write_at(0, &random(4 * BLOCK)).err();
    assert!(
        erreur
            .as_ref()
            .is_some_and(|e| e.to_string().contains("short USB transfer")),
        "{erreur:?}"
    );
    Ok(())
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
    fake.sense_code = ASC_INVALID_OPCODE;
    UsbDisk::open(&mut fake)?.flush()
}

#[test]
fn un_autre_refus_illegal_request_du_vidage_reste_une_erreur() -> Result<(), BlockError> {
    let mut fake = Fake::new(16);
    fake.fault = (0x35, Fault::Check);
    fake.sense_key = SENSE_ILLEGAL_REQUEST;
    fake.sense_code = 0x24;
    assert!(UsbDisk::open(&mut fake)?.flush().is_err());
    Ok(())
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
