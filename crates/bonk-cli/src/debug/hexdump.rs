//! `bonk debug hexdump` : octets bruts d'une image ou d'un disque, par lignes de 16.

use std::io::{self, Write};

use bonk_core::partition::read_range;

use crate::errors::Code;

use super::layout::open_read_only;
use super::{HEXDUMP_MAX, HexdumpArgs, Outcome};

pub(crate) fn hexdump(args: &HexdumpArgs, out: &mut dyn Write) -> io::Result<Outcome> {
    if args.len == 0 || args.len > HEXDUMP_MAX {
        return Ok(Outcome::Failed(
            Code::InvalidArgument,
            format!(
                "longueur invalide : {} (de 1 à {HEXDUMP_MAX} octets)",
                args.len
            ),
        ));
    }
    let device = match open_read_only(&args.image) {
        Ok(device) => device,
        Err((code, message)) => return Ok(Outcome::Failed(code, message)),
    };
    let size = device.size();
    if args.offset >= size {
        return Ok(Outcome::Failed(
            Code::InvalidArgument,
            format!(
                "décalage {} au-delà de la fin de l'image ({size} octets)",
                args.offset
            ),
        ));
    }
    let len = args.len.min(size - args.offset);
    let bytes = match read_range(&*device, args.offset, len) {
        Ok(bytes) => bytes,
        Err(erreur) => {
            return Ok(Outcome::Failed(
                Code::SourceUnreadable,
                format!("lecture impossible : {erreur}"),
            ));
        }
    };
    for line in hexdump_lines(args.offset, &bytes) {
        writeln!(out, "{line}")?;
    }
    if len < args.len {
        writeln!(
            out,
            "(fin de l'image : {len} octets lus sur {} demandés)",
            args.len
        )?;
    }
    Ok(Outcome::Done)
}

/// Lignes de 16 octets : décalage, deux groupes de 8 octets en hexadécimal, texte ASCII.
pub(crate) fn hexdump_lines(start: u64, bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(row, chunk)| {
            let offset = start + (row as u64) * 16;
            let mut hex = String::new();
            for i in 0..16 {
                if i == 8 {
                    hex.push(' ');
                }
                match chunk.get(i) {
                    Some(b) => hex.push_str(&format!("{b:02x} ")),
                    None => hex.push_str("   "),
                }
            }
            let ascii: String = chunk
                .iter()
                .map(|b| {
                    if b.is_ascii_graphic() || *b == b' ' {
                        *b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            format!("{offset:08x}  {hex} |{ascii}|")
        })
        .collect()
}
