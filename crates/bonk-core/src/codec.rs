use std::fmt;
use std::io::{self, BufRead, BufReader, Read};

use ruzstd::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};
use ruzstd::decoding::{BlockDecodingStrategy, FrameDecoder};
use schemars::JsonSchema;
use serde::Serialize;

const GZIP: [u8; 2] = [0x1F, 0x8B];
const XZ: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];
const ZSTD: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
const BZIP2: [u8; 3] = *b"BZh";
const ZIP: [u8; 4] = [b'P', b'K', 0x03, 0x04];

/// Format of an image, from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// Raw image (`.img`, `.iso`, or anything without a compression signature).
    Raw,
    Gzip,
    Xz,
    Zstd,
    /// Recognized but not supported: asks the user to decompress first.
    Bzip2,
    Zip,
}

impl Format {
    /// Content shorter than any signature is raw.
    #[must_use]
    pub fn detect(head: &[u8]) -> Self {
        if head.starts_with(&XZ) {
            Self::Xz
        } else if head.starts_with(&ZSTD) {
            Self::Zstd
        } else if head.starts_with(&GZIP) {
            Self::Gzip
        } else if head.starts_with(&BZIP2) {
            Self::Bzip2
        } else if head.starts_with(&ZIP) {
            Self::Zip
        } else {
            Self::Raw
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Gzip => "gzip",
            Self::Xz => "xz",
            Self::Zstd => "zstd",
            Self::Bzip2 => "bzip2",
            Self::Zip => "zip",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug)]
pub enum CodecError {
    Unsupported(Format),
    Io(io::Error),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(format) => write!(
                f,
                "format {format} not supported: decompress the image with a system tool, \
                 or provide it as .img, .iso, .gz, .xz or .zst"
            ),
            Self::Io(err) => write!(f, "image unreadable: {err}"),
        }
    }
}

impl std::error::Error for CodecError {}

impl From<io::Error> for CodecError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

fn zstd_error(err: impl fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("zstd: {err}"))
}

/// All frames of a zstd stream. `ruzstd::StreamingDecoder` stops after the first one with a normal end, so a
/// `pzstd` file would be half written yet verify. Also checks each frame checksum, which that decoder skips.
struct ZstdFrames<R> {
    source: R,
    decoder: FrameDecoder,
    done: bool,
}

impl<R: BufRead> ZstdFrames<R> {
    fn new(source: R) -> io::Result<Self> {
        let mut frames = Self {
            source,
            decoder: FrameDecoder::new(),
            done: false,
        };
        frames.done = !frames.next_frame()?;
        Ok(frames)
    }

    /// Skips skippable frames; false at the end. Bytes that are not a frame are an error, as for gzip and xz.
    fn next_frame(&mut self) -> io::Result<bool> {
        loop {
            if self.source.fill_buf()?.is_empty() {
                return Ok(false);
            }
            match self.decoder.init(&mut self.source) {
                Ok(()) => return Ok(true),
                Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame {
                    length,
                    ..
                })) => {
                    let wanted = u64::from(length);
                    let mut skipped = (&mut self.source).take(wanted);
                    if io::copy(&mut skipped, &mut io::sink())? != wanted {
                        return Err(io::ErrorKind::UnexpectedEof.into());
                    }
                }
                Err(err) => return Err(zstd_error(err)),
            }
        }
    }

    fn check_sum(&self) -> io::Result<()> {
        match (
            self.decoder.get_checksum_from_data(),
            self.decoder.get_calculated_checksum(),
        ) {
            (Some(read), Some(computed)) if read != computed => {
                Err(zstd_error("wrong frame checksum"))
            }
            _ => Ok(()),
        }
    }
}

impl<R: BufRead> Read for ZstdFrames<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.decoder.can_collect() > 0 {
                return self.decoder.read(buf);
            }
            if self.done {
                return Ok(0);
            }
            if self.decoder.is_finished() {
                self.check_sum()?;
                self.done = !self.next_frame()?;
            } else {
                self.decoder
                    .decode_blocks(&mut self.source, BlockDecodingStrategy::UptoBlocks(1))
                    .map_err(zstd_error)?;
            }
        }
    }
}

pub struct Source {
    pub reader: Box<dyn Read + Send>,
    pub format: Format,
    /// Decompressed size when known in advance: raw images only.
    pub output_len: Option<u64>,
}

/// # Errors
/// [`CodecError::Unsupported`] for bzip2 and zip.
pub fn open(
    input: impl Read + Send + 'static,
    input_len: Option<u64>,
) -> Result<Source, CodecError> {
    let mut buffered = BufReader::with_capacity(64 * 1024, input);
    let format = Format::detect(buffered.fill_buf()?);
    let (reader, output_len): (Box<dyn Read + Send>, Option<u64>) = match format {
        Format::Raw => (Box::new(buffered), input_len),
        Format::Gzip => (Box::new(flate2::read::MultiGzDecoder::new(buffered)), None),
        // Block by block with `lzma-rust2`: `lzma-rs` decodes a whole block in memory, the whole image for a
        // single-block `.img.xz`.
        Format::Xz => (Box::new(lzma_rust2::XzReader::new(buffered, true)), None),
        Format::Zstd => (Box::new(ZstdFrames::new(buffered)?), None),
        Format::Bzip2 | Format::Zip => return Err(CodecError::Unsupported(format)),
    };
    Ok(Source {
        reader,
        format,
        output_len,
    })
}

#[cfg(test)]
pub(crate) mod testing {

    use std::io::Write;

    pub(crate) fn gzip(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let _ = encoder.write_all(data);
        encoder.finish().unwrap_or_default()
    }

    /// Minimal valid xz: header, one stored LZMA2 block, index, footer. Hand-written from the xz
    /// specification.
    pub(crate) fn xz_stored(data: &[u8]) -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            crate::partition::crc32(bytes)
        }
        fn varint(mut n: u64) -> Vec<u8> {
            let mut out = Vec::new();
            while n >= 0x80 {
                out.push((n & 0x7F) as u8 | 0x80);
                n >>= 7;
            }
            out.push(n as u8);
            out
        }
        let mut out = vec![0xFD, b'7', b'z', b'X', b'Z', 0x00];
        let flags = [0x00u8, 0x00]; // check type: none
        out.extend_from_slice(&flags);
        out.extend_from_slice(&crc32(&flags).to_le_bytes());

        // Stored LZMA2 chunks of at most 64 KiB, then 0x00.
        let mut lzma2 = Vec::new();
        for (i, chunk) in data.chunks(1 << 16).enumerate() {
            lzma2.push(if i == 0 { 0x01 } else { 0x02 });
            lzma2.extend_from_slice(&((chunk.len() - 1) as u16).to_be_bytes());
            lzma2.extend_from_slice(chunk);
        }
        lzma2.push(0x00);

        // Block header: size, one LZMA2 filter (id 0x21, one property byte).
        let mut header = vec![0x00, 0x00, 0x21, 0x01, 0x16];
        while (header.len() + 4) % 4 != 0 {
            header.push(0x00);
        }
        header[0] = u8::try_from((header.len() + 4) / 4 - 1).unwrap_or(0);
        let header_crc = crc32(&header);
        let block_start = out.len();
        out.extend_from_slice(&header);
        out.extend_from_slice(&header_crc.to_le_bytes());
        out.extend_from_slice(&lzma2);
        // Unpadded size: header with its checksum, and data.
        let unpadded = out.len() - block_start;
        while (out.len() - block_start) % 4 != 0 {
            out.push(0x00);
        }
        let index_start = out.len();
        out.push(0x00);
        out.extend(varint(1));
        out.extend(varint(unpadded as u64));
        out.extend(varint(data.len() as u64));
        while (out.len() - index_start) % 4 != 0 {
            out.push(0x00);
        }
        let index_crc = crc32(&out[index_start..]);
        out.extend_from_slice(&index_crc.to_le_bytes());
        let backward = u32::try_from((out.len() - index_start) / 4 - 1).unwrap_or(0);
        let mut footer = Vec::new();
        footer.extend_from_slice(&backward.to_le_bytes());
        footer.extend_from_slice(&flags);
        out.extend_from_slice(&crc32(&footer).to_le_bytes());
        out.extend_from_slice(&footer);
        out.extend_from_slice(b"YZ");
        out
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::testing::{gzip, xz_stored};
    use super::*;

    fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 7 % 251) as u8).collect()
    }

    fn read_all(source: &mut Source) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        source.reader.read_to_end(&mut out)?;
        Ok(out)
    }

    #[test]
    fn formats_reconnus_par_leurs_premiers_octets() {
        assert_eq!(
            Format::detect(&[0xFD, b'7', b'z', b'X', b'Z', 0, 1]),
            Format::Xz
        );
        assert_eq!(Format::detect(&[0x28, 0xB5, 0x2F, 0xFD, 0]), Format::Zstd);
        assert_eq!(Format::detect(&[0x1F, 0x8B, 8]), Format::Gzip);
        assert_eq!(Format::detect(b"BZh91AY"), Format::Bzip2);
        assert_eq!(Format::detect(b"PK\x03\x04"), Format::Zip);
        assert_eq!(Format::detect(b"\xEB\x63\x90MSDOS"), Format::Raw);
        assert_eq!(Format::detect(&[]), Format::Raw);
        assert_eq!(Format::detect(&[0x1F]), Format::Raw);
    }

    #[test]
    fn image_brute_lue_telle_quelle_avec_sa_taille() -> Result<(), Box<dyn std::error::Error>> {
        let data = sample(10_000);
        let mut source = open(Cursor::new(data.clone()), Some(data.len() as u64))?;
        assert_eq!(source.format, Format::Raw);
        assert_eq!(source.output_len, Some(10_000));
        assert_eq!(read_all(&mut source)?, data);
        Ok(())
    }

    #[test]
    fn gzip_decompresse_en_flux_et_compte_l_entree() -> Result<(), Box<dyn std::error::Error>> {
        let data = sample(300_000);
        let packed = gzip(&data);
        let packed_len = packed.len() as u64;
        let mut source = open(Cursor::new(packed), Some(packed_len))?;
        assert_eq!(source.format, Format::Gzip);
        assert_eq!(source.output_len, None);
        assert_eq!(read_all(&mut source)?, data);
        Ok(())
    }

    #[test]
    fn gzip_concatene_est_lu_en_entier() -> Result<(), Box<dyn std::error::Error>> {
        let (a, b) = (sample(1000), sample(2000));
        let mut packed = gzip(&a);
        packed.extend(gzip(&b));
        let mut source = open(Cursor::new(packed), None)?;
        let mut expected = a;
        expected.extend(b);
        assert_eq!(read_all(&mut source)?, expected);
        Ok(())
    }

    #[test]
    fn xz_decompresse_en_flux() -> Result<(), Box<dyn std::error::Error>> {
        let data = sample(200_000);
        let mut source = open(Cursor::new(xz_stored(&data)), None)?;
        assert_eq!(source.format, Format::Xz);
        assert_eq!(read_all(&mut source)?, data);
        Ok(())
    }

    #[test]
    fn xz_tronque_donne_une_erreur() -> Result<(), Box<dyn std::error::Error>> {
        let data = sample(200_000);
        let mut packed = xz_stored(&data);
        packed.truncate(packed.len() - 40);
        let mut source = open(Cursor::new(packed), None)?;
        let mut out = Vec::new();
        assert!(source.reader.read_to_end(&mut out).is_err());
        assert!(data.starts_with(&out));
        Ok(())
    }

    #[test]
    fn xz_lecture_partielle_puis_abandon() -> Result<(), Box<dyn std::error::Error>> {
        let data = sample(3_000_000);
        let mut source = open(Cursor::new(xz_stored(&data)), None)?;
        let mut first = [0u8; 100];
        source.reader.read_exact(&mut first)?;
        assert_eq!(&first[..], &data[..100]);
        drop(source);
        Ok(())
    }

    #[test]
    fn gzip_corrompu_est_une_erreur_de_lecture() -> Result<(), Box<dyn std::error::Error>> {
        let mut packed = gzip(&sample(50_000));
        let middle = packed.len() / 2;
        packed[middle] ^= 0xFF;
        packed.truncate(packed.len() - 8);
        let mut source = open(Cursor::new(packed), None)?;
        assert!(read_all(&mut source).is_err());
        Ok(())
    }

    #[test]
    fn bzip2_et_zip_sont_refuses_avec_un_message_utile() {
        for (head, format) in [
            (&b"BZh91AY&SY"[..], Format::Bzip2),
            (&b"PK\x03\x04rest"[..], Format::Zip),
        ] {
            let error = open(Cursor::new(head.to_vec()), None).err();
            assert!(matches!(error, Some(CodecError::Unsupported(f)) if f == format));
            assert!(error.is_some_and(|e| e.to_string().contains("decompress the image")));
        }
    }

    fn zstd(data: &[u8]) -> Vec<u8> {
        ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest)
    }

    /// Output of `pzstd` or `cat a.zst b.zst`: every frame belongs to the image.
    #[test]
    fn zstd_a_plusieurs_trames_est_lu_en_entier() -> Result<(), Box<dyn std::error::Error>> {
        let (a, b) = (sample(100_000), sample(70_000));
        let mut packed = zstd(&a);
        packed.extend(zstd(&b));
        let mut source = open(Cursor::new(packed), None)?;
        let mut expected = a;
        expected.extend(b);
        assert_eq!(read_all(&mut source)?, expected);
        Ok(())
    }

    #[test]
    fn zstd_saute_une_trame_ignorable() -> Result<(), Box<dyn std::error::Error>> {
        let (a, b) = (sample(1000), sample(2000));
        let mut packed = zstd(&a);
        packed.extend(0x184D_2A50u32.to_le_bytes());
        packed.extend(5u32.to_le_bytes());
        packed.extend(b"xxxxx");
        packed.extend(zstd(&b));
        let mut source = open(Cursor::new(packed), None)?;
        let mut expected = a;
        expected.extend(b);
        assert_eq!(read_all(&mut source)?, expected);
        Ok(())
    }

    #[test]
    fn zstd_suivi_de_donnees_est_une_erreur_comme_gzip_et_xz()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut packed = zstd(&sample(1000));
        packed.extend(b"GARBAGE-GARBAGE");
        let mut source = open(Cursor::new(packed), None)?;
        assert!(read_all(&mut source).is_err());
        Ok(())
    }

    #[test]
    fn zstd_a_somme_fausse_est_une_erreur() -> Result<(), Box<dyn std::error::Error>> {
        let data = b"bonk";
        let mut packed = vec![0x28, 0xB5, 0x2F, 0xFD, 0x24, data.len() as u8];
        packed.extend([(data.len() as u8) << 3 | 1, 0, 0]);
        packed.extend(data);
        packed.extend([0, 0, 0, 0]);
        let mut source = open(Cursor::new(packed), None)?;
        assert!(read_all(&mut source).is_err());
        Ok(())
    }

    #[test]
    fn zstd_tronque_est_une_erreur() -> Result<(), Box<dyn std::error::Error>> {
        let mut packed = zstd(&sample(300_000));
        packed.truncate(packed.len() * 3 / 4);
        let mut source = open(Cursor::new(packed), None)?;
        assert!(read_all(&mut source).is_err());
        Ok(())
    }

    #[test]
    fn zstd_signature_invalide_apres_le_magic_est_une_erreur() {
        let error = open(
            Cursor::new(vec![0x28, 0xB5, 0x2F, 0xFD, 0xFF, 0xFF, 0xFF]),
            None,
        )
        .err();
        assert!(matches!(error, Some(CodecError::Io(_))));
    }
}
