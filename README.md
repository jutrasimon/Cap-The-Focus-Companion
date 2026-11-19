# Cap · The Focus Companion

Petite app Windows pour travailler avec intention : sessions de focus, dossiers en cours, jauges de temps par site, rappels et stats. Tout reste sur ton ordinateur.

*A small Windows app for intentional work: focus sessions, active projects, per-site time gauges, reminders and stats. Everything stays on your computer.*

## Installer

1. Télécharge `Cap_x.y.z_x64-setup.exe` dans les [Releases](https://github.com/jutrasimon/Cap-The-Focus-Companion/releases/latest) et lance-le. Cap se met ensuite à jour tout seul.
2. Installe l'extension dans chaque navigateur :
   - **Chrome, Edge, Brave, Vivaldi, Opera** : décompresse `cap-extension-chromium.zip`, puis page des extensions > *Mode développeur* > *Charger l'extension non empaquetée*.
   - **Firefox** : `cap-extension-firefox.zip`, à faire signer une fois sur addons.mozilla.org (*Developer Hub* > *Submit a New Add-on* > *On your own*).

Ctrl+Alt+F ouvre ou réduit Cap. Ctrl+Alt+R ajoute un rappel unique. Les deux se changent dans les réglages.

## Fonctions

- **Focus** : durée exacte, tous les sites actifs bloqués pendant la session.
- **Dossiers** : post-it avec une jauge d'activité qui baisse chaque jour. À zéro, le dossier part au cimetière.
- **Sites** : une jauge par site qui se vide quand tu y es et se recharge sinon. À sec, le site est bloqué.
- **Rappels** : récurrents ou uniques, à confirmer. Deux ratés de suite mettent tout en pause.
- **Stats** : focus, sites, rappels, dossiers, historique.
- **Langues** : français et anglais intégrés, importe la tienne. Voir [LOCALIZATION.md](LOCALIZATION.md).

## Développer

Prérequis : Rust stable, Node 20.

```
npm install
npm run tauri dev
```

`src-tauri/` : cœur Rust (Tauri 2). `ui/` : interface (HTML/CSS/JS sans compilation). `locales/` : textes, une langue par fichier. `extension/` : extension Chromium (MV3). `firefox/` : manifeste Firefox. L'extension parle à Cap sur `127.0.0.1:47615`.

## Publier une version

```
python scripts/bump.py 1.4.0
git commit -am "v1.4.0 : ce qui change"
git push
```

Un commit sur `main` dont le message commence par `v` + numéro de version déclenche la publication.

GitHub Actions compile l'installateur, le signe pour la mise à jour automatique et publie la version. Le secret `TAURI_SIGNING_PRIVATE_KEY` doit être défini dans les réglages du dépôt.

## Licence

MIT
