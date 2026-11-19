# Traduire Cap · Translating Cap

Cap est entièrement traduisible. Chaque langue est un seul fichier JSON dans `locales/`.
*Cap is fully translatable. Each language is a single JSON file in `locales/`.*

## Ajouter une langue · Add a language

1. Copie `locales/en.json` vers un nouveau fichier, par exemple `es.json`.
   *Copy `locales/en.json` to a new file, for example `es.json`.*
2. Change les deux lignes d'identité :
   *Change the two identity lines:*
   ```json
   "_meta.code": "es",
   "_meta.name": "Español",
   "_meta.locale": "es-ES",
   ```
3. Traduis les valeurs, jamais les clés. Garde les variables entre accolades telles quelles : `{n}`, `{h}`, `{m}`, `{s}`, `{v}`.
   *Translate the values, never the keys. Keep variables in braces as they are: `{n}`, `{h}`, `{m}`, `{s}`, `{v}`.*
4. Dans Cap : **Réglages > Langue > Importer** et choisis ton fichier. Il est copié dans le dossier des langues et devient sélectionnable.
   *In Cap: **Settings > Language > Import** and pick your file. It is copied to the languages folder and becomes selectable.*

Une clé absente retombe sur l'anglais : une traduction partielle fonctionne déjà.
*A missing key falls back to English, so a partial translation already works.*

Le dossier des langues (bouton 📁 à côté de Langue) contient aussi des copies modifiables de `fr.json` et `en.json`.
*The languages folder (📁 button next to Language) also holds editable copies of `fr.json` and `en.json`.*

## Partager · Share

Ouvre une *pull request* qui ajoute ton fichier dans `locales/` : il sera intégré à la prochaine version.
*Open a pull request adding your file to `locales/`: it will ship in the next version.*

## Repères · Key prefixes

| Préfixe | Où |
|---|---|
| `fmt.`, `unit.` | Formats de durée et unités · duration formats and units |
| `hdr.`, `tab.` | Barre du haut et onglets · title bar and tabs |
| `focus.`, `dos.`, `site.`, `rem.`, `st.`, `pf.`, `set.` | Onglets Focus, Dossiers, Sites, Rappels, Stats, Performance, Réglages |
| `qa.`, `mini.`, `nudge.`, `tray.` | Actions rapides, bulle, notifications, menu de l'icône |
| `ext.` | Extension de navigateur · browser extension |
