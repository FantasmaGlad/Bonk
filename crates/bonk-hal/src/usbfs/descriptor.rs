use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageInterface {
    pub number: u8,
    pub endpoint_in: u8,
    pub endpoint_out: u8,
}

/// First bulk-only SCSI interface; a UAS-only key is not supported. Truncated descriptors give nothing rather
/// than an out-of-bounds read.
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

/// Text of a string descriptor (UTF-16LE after a two-byte header); `None` when malformed or empty.
pub(super) fn string_of(descriptor: &[u8]) -> Option<String> {
    let length = usize::from(*descriptor.first()?).min(descriptor.len());
    if length < 2 || descriptor.get(1) != Some(&(DESCRIPTOR_STRING as u8)) {
        return None;
    }
    let units: Vec<u16> = descriptor[2..length]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let text = String::from_utf16(&units).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}
