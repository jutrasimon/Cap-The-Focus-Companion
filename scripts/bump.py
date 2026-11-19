"""Change la version partout : python scripts/bump.py 1.4.0"""
import json, re, sys
from pathlib import Path

v = sys.argv[1].lstrip("v")
root = Path(__file__).resolve().parent.parent
for f in ["src-tauri/tauri.conf.json", "package.json", "extension/manifest.json", "firefox/manifest.json"]:
    p = root / f
    data = json.loads(p.read_text(encoding="utf-8"))
    data["version"] = v
    p.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
cargo = root / "src-tauri/Cargo.toml"
cargo.write_text(re.sub(r'(?m)^version = "[^"]+"', f'version = "{v}"', cargo.read_text(encoding="utf-8"), count=1), encoding="utf-8")
print(f"Version {v}. Ensuite : git commit -am 'v{v} : ...' && git push   (un message qui commence par v{v} publie la version)")
