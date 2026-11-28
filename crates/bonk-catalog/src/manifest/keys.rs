/// Current key, then an offline backup that outlives its loss without a new release.
pub const TRUSTED_KEYS: [[u8; 32]; 2] = parse(include_str!("trusted-keys.txt"));

const fn nibble(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        _ => panic!("trusted-keys.txt: lowercase hexadecimal only"),
    }
}

const fn parse<const N: usize>(text: &str) -> [[u8; 32]; N] {
    let bytes = text.as_bytes();
    let mut keys = [[0u8; 32]; N];
    let (mut key, mut digit, mut at) = (0, 0, 0);
    while at < bytes.len() {
        if bytes[at] == b'\n' {
            assert!(
                digit == 64,
                "trusted-keys.txt: 64 hexadecimal digits per key"
            );
            key += 1;
            digit = 0;
        } else {
            assert!(key < N, "trusted-keys.txt: more keys than Bonk trusts");
            let value = nibble(bytes[at]);
            keys[key][digit / 2] |= if digit % 2 == 0 { value << 4 } else { value };
            digit += 1;
        }
        at += 1;
    }
    assert!(
        key == N && digit == 0,
        "trusted-keys.txt: one key per line, each line ended"
    );
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_ligne_donne_une_cle_dans_son_ordre() {
        let hex = |key: &[u8; 32]| key.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let lines: Vec<&str> = include_str!("trusted-keys.txt").lines().collect();
        assert_eq!(TRUSTED_KEYS.map(|key| hex(&key)).to_vec(), lines);
        assert_eq!(TRUSTED_KEYS[0][..4], [0x0a, 0xa9, 0xf4, 0x66]);
        assert_eq!(TRUSTED_KEYS[1][31], 0xee);
    }
}
