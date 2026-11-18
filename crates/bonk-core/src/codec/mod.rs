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
/// xz presets use at most a 64 MiB dictionary; the header alone must not make the decoder allocate gigabytes.
const XZ_MEMORY_KIB: u32 = 256 * 1024;
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
        Format::Xz => (
            Box::new(lzma_rust2::XzReader::new_mem_limit(
                buffered,
                true,
                XZ_MEMORY_KIB,
            )),
            None,
        ),
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
pub(crate) mod testing;
#[cfg(test)]
mod tests;
