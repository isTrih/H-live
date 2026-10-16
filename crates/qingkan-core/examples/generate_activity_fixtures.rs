//! Deterministically regenerate the public synthetic wire fixtures.
use std::{fs, path::PathBuf};
fn var(mut n: u64) -> Vec<u8> {
    let mut v = vec![];
    while n > 127 {
        v.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    v.push(n as u8);
    v
}
fn n(t: u64, v: u64) -> Vec<u8> {
    [var(t << 3), var(v)].concat()
}
fn b(t: u64, v: &[u8]) -> Vec<u8> {
    [var(t << 3 | 2), var(v.len() as u64), v.to_vec()].concat()
}
fn s(t: u64, v: &str) -> Vec<u8> {
    b(t, v.as_bytes())
}
fn main() -> std::io::Result<()> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let common = |message| [n(2, message), n(3, 100000012000), n(4, 1800000000)].concat();
    let lottery = [
        b(1, &common(101)),
        n(2, 100000012000),
        s(3, "LotteryInfoSyncData"),
        n(4, 1),
        b(
            5,
            &[
                n(1, 201),
                n(2, 2),
                n(6, 5),
                n(7, 1800000000),
                n(8, 1800000060),
            ]
            .concat(),
        ),
    ]
    .concat();
    let preview = [
        b(
            1,
            &[
                s(1, "synthetic-packet-1"),
                n(2, 1800000000),
                n(3, 1800000060),
                s(6, "diamond"),
                s(8, "合成红包"),
                n(9, 100),
                n(11, 5),
            ]
            .concat(),
        ),
        b(2, &common(102)),
    ]
    .concat();
    fs::write(directory.join("lottery_sync_synthetic.bin"), lottery)?;
    fs::write(directory.join("red_packet_preview_synthetic.bin"), preview)?;
    Ok(())
}
