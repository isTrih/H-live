/// Only numeric room identifiers can reach the operating system URL opener.
pub fn live_room_url(rid: &str) -> Result<String, &'static str> {
    if rid.is_empty() || rid.len() > 25 || !rid.bytes().all(|value| value.is_ascii_digit()) {
        return Err("直播间 ID 须为 1 至 25 位数字");
    }
    Ok(format!("https://live.douyin.com/{rid}"))
}

#[cfg(test)]
mod tests {
    use super::live_room_url;

    #[test]
    fn preserves_long_numeric_identifiers_without_number_conversion() {
        for value in ["0", "00123", "1234567890123456789012345"] {
            assert_eq!(
                live_room_url(value).unwrap(),
                format!("https://live.douyin.com/{value}")
            );
        }
    }

    #[test]
    fn rejects_urls_shell_metacharacters_unicode_digits_and_bad_lengths() {
        for value in [
            "",
            "12345678901234567890123456",
            "１２３",
            "١٢٣",
            "1&calc",
            "1;calc",
            " 123",
            "123 ",
            "123\n",
            "1\0",
            "../1",
            "https://example.com",
            "1?next=example.com",
            "1#x",
        ] {
            assert!(live_room_url(value).is_err(), "accepted {value:?}");
        }
    }
}
