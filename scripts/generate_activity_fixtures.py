"""Generate synthetic activity fixtures; never fetch or inspect live data."""
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "native_tests"))
from activity_fixtures import number as n, text as s, blob as b


def generate():
    room = 100000012000
    common = lambda message: n(2, message) + n(3, room) + n(4, 1800000000)
    lottery = (b(1, common(101)) + n(2, room) + s(3, "LotteryInfoSyncData")
               + n(4, 1) + b(5, n(1, 201) + n(2, 2) + n(6, 5)
                                      + n(7, 1800000000) + n(8, 1800000060)))
    preview = (b(1, s(1, "synthetic-packet-1") + n(2, 1800000000)
                 + n(3, 1800000060) + s(6, "diamond") + s(8, "合成红包")
                 + n(9, 100) + n(11, 5)) + b(2, common(102)))
    fixtures = ROOT / "crates/qingkan-core/tests/fixtures"
    fixtures.mkdir(parents=True, exist_ok=True)
    (fixtures / "lottery_sync_synthetic.bin").write_bytes(lottery)
    (fixtures / "red_packet_preview_synthetic.bin").write_bytes(preview)


if __name__ == "__main__":
    generate()
