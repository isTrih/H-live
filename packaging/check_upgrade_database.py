"""Copy a historical synthetic database and compare its rows after an upgrade."""
from pathlib import Path
import argparse
import hashlib
import json
import sqlite3

ROOT = Path(__file__).resolve().parents[1]


def workspace_file(value):
    path = Path(value).resolve()
    if not path.is_relative_to(ROOT / "build"):
        raise ValueError("Upgrade fixtures must remain under the workspace build directory")
    return path


def snapshot(path):
    with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True) as db:
        db.execute("BEGIN")
        tables = {}
        for table in ("anchors", "sessions", "messages", "sqlite_sequence"):
            order = "name" if table == "sqlite_sequence" else "rowid"
            rows = db.execute(f'SELECT * FROM "{table}" ORDER BY {order}').fetchall()
            encoded = json.dumps(rows, ensure_ascii=False, separators=(",", ":")).encode()
            tables[table] = {"rows": len(rows), "sha256": hashlib.sha256(encoded).hexdigest()}
        return {"user_version": db.execute("PRAGMA user_version").fetchone()[0], "tables": tables,
                "indexes": [list(row) for row in db.execute("SELECT name,sql FROM sqlite_schema WHERE type='index' AND tbl_name<>'session_monitor_durations' AND tbl_name NOT LIKE 'quality_%' AND name<>'messages_quality_session' AND name NOT IN ('messages_type_recent','messages_room_recent','messages_room_type_recent','messages_lottery_draw') ORDER BY name")],
                "lottery_dedupe_index": any(row[1]=='messages_lottery_draw' and row[2]==1 for row in db.execute('PRAGMA index_list(messages)')),
                "duration_table": bool(db.execute("SELECT 1 FROM sqlite_schema WHERE type='table' AND name='session_monitor_durations'").fetchone()),
                "foreign_keys": [list(row) for row in db.execute("PRAGMA foreign_key_list(messages)")],
                "foreign_key_violations": [list(row) for row in db.execute("PRAGMA foreign_key_check")],
                "integrity": [list(row) for row in db.execute("PRAGMA integrity_check")]}


def compatible(before, after):
    # 0.2.1 widens the message CHECK from seven types to eight. It deliberately
    # upgrades user_version; comparing the complete dictionaries would reject a
    # correct migration. Every stored value/sequence/index/FK must still match.
    return (before["user_version"] in (1, 2, 3) and after["user_version"] == 3
            and all(before[key] == after[key] for key in ("tables", "indexes", "foreign_keys"))
            and after["duration_table"] and after["lottery_dedupe_index"] and not after["foreign_key_violations"]
            and after["integrity"] == [["ok"]])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "check"))
    parser.add_argument("--database", required=True)
    parser.add_argument("--report", required=True)
    parser.add_argument("--source")
    args = parser.parse_args()
    database, report_path = workspace_file(args.database), workspace_file(args.report)
    if args.action == "prepare":
        source = workspace_file(args.source)
        if database.exists():
            raise ValueError("Refusing to overwrite an existing upgrade database")
        database.parent.mkdir(parents=True, exist_ok=True)
        with sqlite3.connect(source.as_uri() + "?mode=ro", uri=True) as old:
            with sqlite3.connect(database) as new:
                old.backup(new)
        report = {"ok": False, "source": str(source), "database": str(database), "before": snapshot(database)}
        assert report["before"]["tables"]["messages"]["rows"] > 0
    else:
        report = json.loads(report_path.read_text(encoding="utf-8"))
        report["after"] = json.loads(json.dumps(snapshot(database)))
        report["checks"] = {"records_and_identity_preserved": report["before"]["tables"] == report["after"]["tables"],
                            "schema_v3": report["after"]["user_version"] == 3}
        report["ok"] = compatible(report["before"], report["after"])
        report["downgrade_boundary"] = "Schema v3 requires 0.2.2 or newer; older applications reject the migrated archive."
        assert report["ok"], "Upgrade changed the synthetic archive rows"
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"action": args.action, "ok": report["ok"], "report": str(report_path)}))


if __name__ == "__main__":
    main()
