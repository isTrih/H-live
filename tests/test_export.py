import csv
from pathlib import Path
import tempfile
import unittest
import re
from xml.etree import ElementTree as ET
from zipfile import ZipFile

from archive_store import ArchiveStore
import message_export as export


NS = {"s": "http://schemas.openxmlformats.org/spreadsheetml/2006/main"}


def sheet_values(path, sheet=1):
    with ZipFile(path) as archive:
        document = ET.fromstring(archive.read(f"xl/worksheets/sheet{sheet}.xml"))
    return [[cell.find("s:is/s:t", NS).text or "" for cell in row]
            for row in document.findall("s:sheetData/s:row", NS)]


def sheet_names(path):
    with ZipFile(path) as archive:
        workbook = ET.fromstring(archive.read("xl/workbook.xml"))
    return [re.sub(r"_x([0-9a-fA-F]{4})_", lambda match: chr(int(match[1], 16)), row.attrib["name"])
            for row in workbook.find("s:sheets", NS)]


class ExportTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.store = ArchiveStore(self.root)
        self.serial = 0

    def tearDown(self):
        self.store.close()
        self.temporary.cleanup()

    def add(self, **changes):
        self.serial += 1
        record = {
            "anchor_id": "123456789012345678901", "anchor_name": "主播甲😀",
            "web_rid": "123456", "session_id": "987654321098765432109",
            "sender_id": "00123456789012345678901234567890", "sender_name": "观众",
            "message_type": "chat", "content": "你好，世界🌏", "message_time": "2027-01-15T10:11:12.123+08:00",
            "received_at": "2027-01-15T10:11:13+08:00", "time_source": "server",
            "message_id": str(self.serial),
        }
        record.update(changes)
        if not record["message_time"]:
            record["time_source"] = "missing"
        self.store.record_message(record)
        return record

    def run_export(self, suffix="csv", **kwargs):
        target = self.root / ("export." + suffix)
        report = export.export_month(self.root, ["123456789012345678901"], "2027-01", target, **kwargs)
        return target, report

    def csv_rows(self, path):
        with path.open(encoding="utf-8-sig", newline="") as stream:
            return list(csv.reader(stream))

    def test_month_boundaries_cross_year_midnight_and_server_time_priority(self):
        self.add(content="december", message_time="2026-12-31T15:59:59.999+00:00")
        self.add(content="january-first", message_time="2026-12-31T16:00:00+00:00")
        self.add(content="january-last", message_time="2027-01-31T15:59:59.999+00:00")
        self.add(content="february", message_time="2027-01-31T16:00:00+00:00")
        self.add(content="server-month", received_at="2027-02-01T00:00:00+08:00")
        self.add(content="missing-time", message_time="", received_at="2026-12-31T16:00:00+00:00", sender_id="")
        self.add(content="missing-outside", message_time="", received_at="2027-02-01T00:00:00+08:00")
        self.add(content="other-anchor", anchor_id="999", anchor_name="乙")
        path, report = self.run_export()
        rows = self.csv_rows(path)
        self.assertEqual(report["row_count"], 4)
        self.assertEqual({row[5] for row in rows[1:]}, {"january-first", "january-last", "server-month", "missing-time"})
        missing = next(row for row in rows[1:] if row[5] == "missing-time")
        self.assertEqual(missing[2], "")
        self.assertEqual(missing[6], "")
        self.assertEqual(report["missing_sender_count"], 1)
        self.assertEqual(report["missing_time_count"], 1)
        first = next(row for row in rows[1:] if row[5] == "january-first")
        self.assertEqual(first[6], "2027-01-01 00:00:00+08:00")

    def test_multiple_selected_anchors_and_type_filter(self):
        self.add(content="first", message_type="chat")
        self.add(content="second", anchor_id="999", anchor_name="乙", message_type="gift")
        self.add(content="unselected", anchor_id="888", anchor_name="丙")
        self.add(content="filtered", message_type="like")
        path = self.root / "chosen.csv"
        report = export.export_month(self.root, ["123456789012345678901", "999"], "2027-01", path, types=["chat", "gift"])
        self.assertEqual(report["row_count"], 2)
        self.assertEqual({row[5] for row in self.csv_rows(path)[1:]}, {"first", "second"})

    def test_csv_exact_long_id_chinese_emoji_and_bom(self):
        record = self.add(content='中文😀,"引号"\r\n换行')
        path, report = self.run_export()
        self.assertTrue(path.read_bytes().startswith(b"\xef\xbb\xbf"))
        rows = self.csv_rows(path)
        self.assertEqual(rows[0], list(export.HEADERS))
        self.assertEqual(rows[1][1], record["session_id"])
        self.assertEqual(rows[1][2], record["sender_id"])
        self.assertEqual(rows[1][5], record["content"])
        self.assertEqual(report["formula_protected_count"], 0)

    def test_csv_dangerous_formula_prefixes_are_inert_and_counted(self):
        values = ["=1+1", "+SUM(1,2)", "-2+3", "@SUM(A1)", "  =1", "\tword", "\n=1", "\ufeff\t =1", "\x01=1", "\u00a0@SUM(A1)", "safe", "'already-text"]
        for value in values:
            self.add(content=value)
        path, report = self.run_export()
        contents = [row[5] for row in self.csv_rows(path)[1:]]
        self.assertEqual(contents, ["'" + value for value in values[:10]] + values[10:])
        self.assertEqual(report["formula_protected_count"], 10)

    def test_xlsx_has_seven_string_columns_freeze_filter_styles_and_no_formulas(self):
        record = self.add(content='=HYPERLINK("https://example.invalid") & <中文> 😀\r\nnext')
        path, report = self.run_export("xlsx")
        rows = sheet_values(path)
        self.assertEqual(rows[0], list(export.HEADERS))
        self.assertEqual(rows[1][1:3], [record["session_id"], record["sender_id"]])
        self.assertEqual(rows[1][3], record["sender_name"])
        self.assertEqual(rows[1][5], record["content"])
        self.assertEqual(report["formula_protected_count"], 0)
        self.assertEqual(report["sheet_count"], 1)
        with ZipFile(path) as archive:
            self.assertIsNone(archive.testzip())
            for name in archive.namelist():
                if name.endswith((".xml", ".rels")):
                    ET.fromstring(archive.read(name))
            root = ET.fromstring(archive.read("xl/worksheets/sheet1.xml"))
            self.assertEqual(root.find("s:sheetViews/s:sheetView/s:pane", NS).attrib["state"], "frozen")
            self.assertEqual(root.find("s:autoFilter", NS).attrib["ref"], "A1:G2")
            self.assertTrue(all(cell.attrib["t"] == "inlineStr" for cell in root.findall(".//s:c", NS)))
            self.assertEqual(root.findall(".//s:f", NS), [])
            self.assertIn("xl/styles.xml", archive.namelist())

    def test_xlsx_literal_ooxml_escape_stays_literal(self):
        self.add(content="literal _x0041_ _x005F_ sequence")
        path, _ = self.run_export("xlsx")
        self.assertEqual(sheet_values(path)[1][5], "literal _x005F_x0041_ _x005F_x005F_ sequence")


    def test_zero_rows_produces_header_in_both_formats(self):
        for kind in ("csv", "xlsx"):
            path, report = self.run_export(kind)
            rows = self.csv_rows(path) if kind == "csv" else sheet_values(path)
            self.assertEqual(rows, [list(export.HEADERS)])
            self.assertEqual(report["row_count"], 0)

    def test_legacy_jsonl_never_enters_export(self):
        legacy = self.root / "messages" / "123456789012345678901"
        legacy.mkdir(parents=True)
        (legacy / "2027-01-15.jsonl").write_text('{"type":"chat","text":"legacy without real session"}\n', encoding="utf-8")
        path, report = self.run_export()
        self.assertEqual(report["row_count"], 0)
        self.assertEqual(len(self.csv_rows(path)), 1)
        self.assertTrue((legacy / "2027-01-15.jsonl").exists())

    def test_bad_month_type_anchor_and_format_validation_preserves_destination(self):
        path = self.root / "export.csv"
        path.write_text("original", encoding="utf-8")
        for month in ("2027-1", "2027-00", "2027-13", "0000-01", "2027-01-01", None):
            with self.assertRaises(ValueError):
                export.export_month(self.root, ["123"], month, path)
        with self.assertRaises(ValueError):
            export.export_month(self.root, [], "2027-01", path)
        with self.assertRaises(ValueError):
            export.export_month(self.root, "123", "2027-01", path)
        with self.assertRaises(ValueError):
            export.export_month(self.root, ["123"], "2027-01", path, types=["viewers"])
        with self.assertRaises(ValueError):
            export.export_month(self.root, ["123"], "2027-01", path, format="xlsx")
        self.assertEqual(path.read_text(), "original")

    def test_long_or_illegal_xml_text_fails_without_truncation_or_overwrite(self):
        path = self.root / "export.xlsx"
        for index, content in enumerate(("x" * 32768, "before\x01after")):
            with self.subTest(content_length=len(content)):
                anchor_id = str(100 + index)
                self.add(content=content, anchor_id=anchor_id)
                path.write_bytes(b"original")
                with self.assertRaisesRegex(export.ExportDataError, "CSV"):
                    export.export_month(self.root, [anchor_id], "2027-01", path)
                self.assertEqual(path.read_bytes(), b"original")
                self.assertEqual(list(self.root.glob(".export.xlsx.*.tmp")), [])

    def test_csv_retains_very_long_messages_and_control_characters(self):
        content = "字" * 40000 + "\x01"
        self.add(content=content)
        path, report = self.run_export()
        self.assertEqual(self.csv_rows(path)[1][5], content)
        self.assertEqual(report["row_count"], 1)




    def test_xlsx_groups_by_selected_anchor_id_and_keeps_selection_order(self):
        self.add(content="甲先", anchor_name="甲", message_time="2027-01-01T00:00:00+08:00")
        self.add(content="乙中", anchor_id="999", anchor_name="乙", message_time="2027-01-02T00:00:00+08:00")
        self.add(content="甲后", anchor_name="甲", message_time="2027-01-03T00:00:00+08:00")
        path = self.root / "multi.xlsx"
        report = export.export_month(self.root, ["999", "123456789012345678901"], "2027-01", path)
        self.assertEqual(sheet_names(path), ["乙", "甲"])
        self.assertEqual([row[5] for row in sheet_values(path, 1)[1:]], ["乙中"])
        self.assertEqual([row[5] for row in sheet_values(path, 2)[1:]], ["甲先", "甲后"])
        self.assertEqual((report["row_count"], report["sheet_count"]), (3, 2))

    def test_xlsx_same_or_case_insensitive_names_never_merge_anchor_ids(self):
        ids = ["1", "2", "3", "4"]
        for anchor_id, name in zip(ids, ["Alice", "alice", "Alice", "ALICE (2)"]):
            self.add(anchor_id=anchor_id, anchor_name=name, content=f"only-{anchor_id}")
        path = self.root / "same.xlsx"
        export.export_month(self.root, ids, "2027-01", path)
        names = sheet_names(path)
        self.assertEqual(len({name.casefold() for name in names}), 4)
        for sheet, anchor_id in enumerate(ids, 1):
            self.assertEqual([row[5] for row in sheet_values(path, sheet)[1:]], [f"only-{anchor_id}"])
        second = self.root / "same-again.xlsx"
        export.export_month(self.root, ids, "2027-01", second)
        self.assertEqual(sheet_names(second), names)

    def test_sheet_names_sanitize_reserved_empty_long_unicode_and_xml_literals(self):
        raw_names = ["' /\\:*?[] '", "History", "history", "", "😀" * 20,
                     "A" * 30 + "'suffix", '名字"&<tag>_x0041_', "x\x01y\nz"]
        ids = [str(index + 1) for index in range(len(raw_names))]
        for anchor_id, name in zip(ids, raw_names):
            # Metadata exists, but this month has no rows: name sanitization must
            # not change archived cells or force invalid XML content into them.
            self.add(anchor_id=anchor_id, anchor_name=name, message_time="2026-12-01T00:00:00+08:00")
        path = self.root / "names.xlsx"
        report = export.export_month(self.root, ids, "2027-01", path)
        names = sheet_names(path)
        self.assertEqual(len(names), len(ids))
        self.assertEqual(names[3], "主播 4")
        self.assertEqual(names[4], "😀" * 15)
        self.assertEqual(names[5], "A" * 30)
        self.assertEqual(names[6], raw_names[6])
        self.assertEqual(report["row_count"], 0)
        for number, name in enumerate(names, 1):
            self.assertTrue(name and name.casefold() != "history")
            self.assertLessEqual(len(name.encode("utf-16-le")) // 2, 31)
            self.assertFalse(re.search(r"[\\/?*:\[\]\x00-\x1f]", name))
            self.assertFalse(name.startswith("'") or name.endswith("'"))
            self.assertEqual(sheet_values(path, number), [list(export.HEADERS)])

    def test_latest_metadata_names_sheets_without_rewriting_historical_row_names(self):
        self.add(anchor_name="旧名字", content="first")
        self.add(anchor_name="另一个历史名字", content="second")
        self.add(anchor_name="当前名字", message_time="2027-02-01T00:00:00+08:00")
        path, report = self.run_export("xlsx")
        self.assertEqual(sheet_names(path), ["当前名字"])
        self.assertEqual([row[0] for row in sheet_values(path)[1:]], ["旧名字", "另一个历史名字"])
        self.assertEqual(report["sheet_count"], 1)

    def test_each_selected_anchor_has_sheet_when_month_or_type_has_no_rows(self):
        self.add(anchor_id="1", anchor_name="只有礼物", message_type="gift")
        self.add(anchor_id="2", anchor_name="其他月份", message_time="2026-12-01T00:00:00+08:00")
        path = self.root / "empty-multi.xlsx"
        report = export.export_month(self.root, ["1", "2", "3"], "2027-01", path, types=["chat"])
        self.assertEqual(sheet_names(path), ["只有礼物", "其他月份", "主播 3"])
        self.assertEqual((report["sheet_count"], report["row_count"]), (3, 0))
        for number in range(1, 4):
            self.assertEqual(sheet_values(path, number), [list(export.HEADERS)])




    def test_sender_name_column_preserves_original_and_missing_names(self):
        sender = '观众😀 "名字" & <x>\r\n下一行'
        self.add(sender_name=sender, content="named")
        self.add(sender_name="", sender_id="12345", content="unnamed")
        for kind in ("csv", "xlsx"):
            path, _ = self.run_export(kind)
            rows = self.csv_rows(path) if kind == "csv" else sheet_values(path)
            self.assertEqual(len(rows[0]), 7)
            self.assertEqual(rows[0][3], "发消息人名字")
            self.assertEqual(rows[1][3], sender)
            self.assertEqual(rows[2][3], "")
            self.assertEqual(rows[2][2], "12345")


if __name__ == "__main__":
    unittest.main()
