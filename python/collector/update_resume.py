"""Atomic, credential-free intent written only by explicit update preparation."""
import json
import os
from pathlib import Path
import re
import tempfile
import uuid

NAME = "update-resume.json"

def valid_version(value):
    return isinstance(value, str) and len(value) <= 80 and re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", value) is not None

def read_intent(directory):
    path = Path(directory) / NAME
    if not path.exists():
        return None
    if path.stat().st_size > 32768:
        raise ValueError("更新恢复记录超出大小限制")
    value = json.loads(path.read_text(encoding="utf-8"))
    if (not isinstance(value, dict) or value.get("format") != 1
        or not valid_version(value.get("from_version")) or not valid_version(value.get("target_version"))
        or not isinstance(value.get("nonce"), str) or not re.fullmatch(r"[0-9a-f]{32}", value["nonce"])
        or not isinstance(value.get("rids"), list) or len(value["rids"]) > 500
        or any(not isinstance(rid, str) or not re.fullmatch(r"[0-9]{1,64}", rid) or not int(rid) for rid in value["rids"])):
        raise ValueError("更新恢复记录无效，已保留原文件")
    return value

def save_intent(directory, current_version, target_version, rids):
    if not valid_version(current_version) or not valid_version(target_version):
        raise ValueError("更新版本无效")
    rids = list(dict.fromkeys(rids))
    if len(rids)>500 or any(not isinstance(rid,str) or not re.fullmatch(r"[0-9]{1,64}",rid) or not int(rid) for rid in rids):
        raise ValueError("更新恢复任务列表无效")
    value = {"format":1,"from_version":current_version,"target_version":target_version,"rids":rids,"nonce":uuid.uuid4().hex}
    path = Path(directory) / NAME
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w",encoding="utf-8",dir=path.parent,prefix=".update-resume-",suffix=".tmp",delete=False) as f:
            temporary=Path(f.name)
            json.dump(value,f,ensure_ascii=False)
            f.flush()
            os.fsync(f.fileno())
        os.replace(temporary,path)
        return value
    finally:
        if temporary is not None and temporary.exists():
            temporary.unlink()

def resume_rids(intent,current_version,configured):
    if not intent or current_version not in (intent["from_version"],intent["target_version"]):
        return []
    return [rid for rid in dict.fromkeys(intent["rids"]) if rid in configured]

def acknowledge(directory,nonce):
    value=read_intent(directory)
    if value is not None and value["nonce"]==nonce:
        (Path(directory)/NAME).unlink()
        return True
    return False
