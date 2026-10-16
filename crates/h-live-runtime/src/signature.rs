//! WSS compatibility transform ported from eplayn's MIT xbogus.c.
//! Copyright (c) 2026 eplayn. See licenses/upstream/Remastered MIT notice.
//! MD5 and RC4 here are wire compatibility, never credential protection.
use md5::{Digest, Md5};
use url::Url;

const ALPHABET: &[u8; 64] = b"Dkdpgh4ZKsQB80/Mfvw36XI1R25-WUAlEi7NLboqYTOPuzmFjJnryx9HVGcaStCe";
const FIELDS: [&str; 13] = [
    "live_id",
    "aid",
    "version_code",
    "webcast_sdk_version",
    "room_id",
    "sub_room_id",
    "sub_channel_id",
    "did_rule",
    "user_unique_id",
    "device_platform",
    "device_type",
    "ac",
    "identity",
];

pub fn canonical(url: &Url) -> String {
    let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
    FIELDS
        .iter()
        .map(|key| {
            format!(
                "{key}={}",
                pairs.get(*key).map(|v| v.as_ref()).unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}
pub fn compute(digest: [u8; 16], header: u8, byte8: u8, random: u8) -> String {
    let q = Md5::digest(digest);
    let mut p = [1, 0, 1, 14, 0x45, 0x3f, q[14], q[15], byte8, 0];
    p[9] = p[..9].iter().fold(0, |a, b| a ^ b);
    let mut s = [0u8; 256];
    for (i, v) in s.iter_mut().enumerate() {
        *v = i as u8;
    }
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + s[i] as usize + random as usize) & 255;
        s.swap(i, j);
    }
    let mut i = 0;
    j = 0;
    for b in &mut p {
        i = (i + 1) & 255;
        j = (j + s[i] as usize) & 255;
        s.swap(i, j);
        *b ^= s[(s[i] as usize + s[j] as usize) & 255];
    }
    let mut raw = [0; 12];
    raw[0] = 0x40 | ((header & 1) << 4);
    raw[1] = random;
    raw[2..].copy_from_slice(&p);
    let mut out = String::with_capacity(16);
    for bytes in raw.as_chunks::<3>().0 {
        let n = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        for shift in [18, 12, 6, 0] {
            out.push(ALPHABET[((n >> shift) & 63) as usize] as char);
        }
    }
    out
}
pub fn sign(url: &Url) -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    compute(
        Md5::digest(canonical(url).as_bytes()).into(),
        rng.random_range(0..2),
        rng.random_range(0..255),
        rng.random_range(0..255),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upstream_zero_vectors() {
        for (input, expected) in [
            ("b5f7f2b3e491718deb69195be3284b1b", "fDpl4KiMGE7XTRYb"),
            ("51b61920a08ead94f29e425711c5ba5c", "fDpl4KiMGEN6PVTm"),
            ("00000000000000000000000000000000", "fDpl4KiMGEihqVPM"),
            ("ffffffffffffffffffffffffffffffff", "fDpl4KiMGEi/CKON"),
            ("0123456789abcdef0123456789abcdef", "fDpl4KiMGENs2KOV"),
        ] {
            let mut raw = [0; 16];
            for i in 0..16 {
                raw[i] = u8::from_str_radix(&input[i * 2..i * 2 + 2], 16).unwrap();
            }
            assert_eq!(compute(raw, 0, 0, 0), expected);
        }
    }
    #[test]
    fn canonical_matches_url_and_keeps_missing_fields() {
        let u = Url::parse(
            "wss://example.com/?room_id=123&user_unique_id=456&identity=audience&aid=6383",
        )
        .unwrap();
        assert_eq!(canonical(&u),"live_id=,aid=6383,version_code=,webcast_sdk_version=,room_id=123,sub_room_id=,sub_channel_id=,did_rule=,user_unique_id=456,device_platform=,device_type=,ac=,identity=audience");
        for h in [0, 1] {
            for r in [0, 1, 127, 254, 255] {
                let s = compute([r; 16], h, 255, r);
                assert_eq!(s.len(), 16);
                assert!(s.bytes().all(|b| ALPHABET.contains(&b)));
            }
        }
    }
}
