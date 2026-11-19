"""Règle dure : aucun texte affiché en dur. Vérifie que
1. chaque clé utilisée existe dans toutes les langues intégrées (fr, en), qui ont exactement les mêmes clés ;
2. les pages HTML n'affichent aucun texte sans data-t ;
3. les scripts et le cœur Rust n'ont pas de phrase en dur (lignes marquées « i18n-ok » exclues).
Usage : python scripts/check_i18n.py   (code de sortie 1 si un problème est trouvé)"""
import json, re, sys
from html.parser import HTMLParser
from pathlib import Path

root = Path(__file__).resolve().parent.parent
errors = []
locales = {c: json.loads((root / f"locales/{c}.json").read_text(encoding="utf-8")) for c in ("fr", "en")}
ref = set(locales["fr"])
for c, d in locales.items():
    for k in sorted(ref ^ set(d)):
        errors.append(f"locales/{c}.json : clé {'manquante' if k not in d else 'en trop'} « {k} »")

# Clés utilisées. Les clés construites (`key.${k}`) sont vérifiées par préfixe.
used, prefixes = set(), set()
pat_js = re.compile(r"""\b(?:t|tx)\(\s*["']([a-zA-Z0-9_.]+)["']""")
pat_js_dyn = re.compile(r"""\b(?:t|tx)\(\s*`([a-zA-Z0-9_.]+)\$\{""")
pat_html = re.compile(r"""data-t(?:-ph|-tip)?="([a-zA-Z0-9_.]+)\"""")
pat_rs = re.compile(r"""\btrn?\(\s*"([a-zA-Z0-9_.]+)\"""")
pat_rs_dyn = re.compile(r"""format!\("([a-zA-Z0-9_]+\.[a-zA-Z0-9_.]*)\{""")
files = list((root / "ui").glob("*.js")) + list((root / "ui").glob("*.html")) + list((root / "extension").glob("*.js")) + [root / "src-tauri/src/main.rs"]
for f in files:
    txt = f.read_text(encoding="utf-8")
    for p in (pat_js, pat_html, pat_rs):
        used.update(p.findall(txt))
    for p in (pat_js_dyn, pat_rs_dyn):
        prefixes.update(p.findall(txt))
for k in sorted(used):
    if k not in ref and not k.startswith("_meta."):
        errors.append(f"clé utilisée mais absente des langues : « {k} »")
for p in sorted(prefixes):
    if not any(k.startswith(p) for k in ref):
        errors.append(f"préfixe de clé sans aucune clé : « {p}… »")

# HTML : texte visible sans data-t
class Scan(HTMLParser):
    def __init__(self, name):
        super().__init__()
        self.name, self.stack = name, []
    def handle_starttag(self, tag, attrs):
        if tag not in ("input", "img", "meta", "link", "br", "circle", "stop", "path", "polyline", "polygon"):
            self.stack.append((tag, dict(attrs)))
    def handle_endtag(self, tag):
        while self.stack:
            if self.stack.pop()[0] == tag:
                break
    def handle_data(self, data):
        text = data.strip()
        if not text or not re.search(r"[A-Za-zÀ-ÿ]{2,}", text):
            return
        if any(t in ("script", "style", "title") for t, _ in self.stack):
            return
        if any("data-t" in a for _, a in self.stack):
            return
        if text in ("Cap",):
            return
        errors.append(f"{self.name} : texte en dur « {text[:40]} »")
for f in list((root / "ui").glob("*.html")) + list((root / "extension").glob("*.html")):
    Scan(f.relative_to(root).as_posix()).feed(f.read_text(encoding="utf-8"))

# Scripts et Rust : phrases en dur (au moins deux mots dans une chaîne)
phrase = re.compile(r"""(["'`])((?:(?!\1).)*?[A-Za-zÀ-ÿ]{2,}[ ’'][A-Za-zÀ-ÿ]{2,}(?:(?!\1).)*?)\1""")
allowed = re.compile(r"^(Segoe|Cascadia|system-ui|sans-serif|[a-z-]+ [a-z-]+(?: [a-z-]+)*)$")
for f in list((root / "ui").glob("*.js")) + list((root / "extension").glob("*.js")) + [root / "src-tauri/src/main.rs"]:
    if f.name in ("strings.js", "icons.js"):
        continue
    for i, line in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
        code = line.split("//")[0] if f.suffix == ".rs" else re.sub(r"(^|\s)//.*$", "", line)
        if "i18n-ok" in line or code.strip().startswith(("*", "///", "//!")):
            continue
        for _, s in phrase.findall(code):
            if "${" in s or "<" in s or s[:1] in "#.[" or allowed.match(s) or re.search(r"[A-Z]:\\\\|https?:|--|\{\}|\{[a-z]+\}", s):
                continue
            if re.search(r"[A-ZÀ-Ý][a-zà-ÿ]+ [a-zà-ÿ]|[à-ÿ]", s):
                errors.append(f"{f.relative_to(root).as_posix()}:{i} : texte en dur « {s[:50]} »")

# extension/strings.js : textes français de secours de l'extension, à garder synchronisés
ext = {k: v for k, v in locales["fr"].items() if k.startswith("ext.")}
gen = "// Textes par défaut (français). Cap envoie ceux de la langue choisie.\n// Généré depuis locales/fr.json par scripts/check_i18n.py --fix.\nglobalThis.CAP_FR = " + json.dumps(ext, ensure_ascii=False, indent=1) + ";\n"
sj = root / "extension/strings.js"
if "--fix" in sys.argv:
    sj.write_text(gen, encoding="utf-8")
elif sj.read_text(encoding="utf-8") != gen:
    errors.append("extension/strings.js n'est pas à jour : python scripts/check_i18n.py --fix")

if errors:
    print("Textes : " + str(len(errors)) + " problème(s)")
    print("\n".join(errors))
    sys.exit(1)
print(f"Textes : OK ({len(ref)} clés, {len(used)} utilisées)")
