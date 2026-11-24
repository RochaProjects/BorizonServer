//! UUIDs as vanilla's four-int form, to and from their text form.
/// Format four big-endian 32-bit words as the canonical dashed UUID text.
pub fn uuid_text(u: [i32; 4]) -> String {
    let hex: String = u.iter().map(|v| format!("{:08x}", *v as u32)).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
/// Parse vanilla UUID text, accepting optional dashes as vanilla does.
pub fn uuid_from_text(s: &str) -> Option<[i32; 4]> {
    let mut hex = [0u8; 32];
    let mut len = 0;
    for b in s.bytes() {
        if b == b'-' {
            continue;
        }
        if !b.is_ascii_hexdigit() || len >= 32 {
            return None;
        }
        hex[len] = b;
        len += 1;
    }
    if len != 32 {
        return None;
    }
    let hex = std::str::from_utf8(&hex).ok()?;
    let mut out = [0i32; 4];
    for (i, v) in out.iter_mut().enumerate() {
        *v = u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).ok()? as i32;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_text_uses_canonical_groups_and_preserves_sign_bits() {
        let value = [0x0123_4567, -0x7654_3211, 0x0123_4567, -0x7654_3211];
        let text = uuid_text(value);
        assert_eq!(text, "01234567-89ab-cdef-0123-456789abcdef");
        assert_eq!(uuid_from_text(&text), Some(value));
    }

    #[test]
    fn uuid_from_text_accepts_undashed_hex() {
        let value = [0, 1, 2, 3];
        let text: String = uuid_text(value).chars().filter(|c| *c != '-').collect();
        assert_eq!(uuid_from_text(&text), Some(value));
    }

    #[test]
    fn uuid_from_text_rejects_invalid_bytes_and_lengths() {
        assert_eq!(uuid_from_text("€€€€€€€€€€aa"), None);
        assert_eq!(uuid_from_text("+0000001-0000-0000-0000-000000000000"), None);
        assert_eq!(uuid_from_text("00000000-0000-0000-0000-00000000000g"), None);
        assert_eq!(uuid_from_text(""), None);
        assert_eq!(uuid_from_text("000000000000000000000000000000000"), None);
    }
}
