# Cap · règles du projet

## Textes : règle dure
- **Aucun texte affiché en dur.** Tout texte visible (interface, infobulles, notifications, menu de l'icône, extension, erreurs) passe par une clé de `locales/*.json`.
  - HTML : `data-t`, `data-t-ph` (placeholder), `data-t-tip` (infobulle).
  - JS : `t("clé", { n })`. Extension : `tx("clé")`. Rust : `tr("clé")`, `trn("clé", n)`.
- Chaque nouvelle clé est ajoutée **dans `fr.json` et `en.json`**, dans le même commit.
- `python scripts/check_i18n.py` doit passer avant tout commit (la publication l'exécute et échoue sinon). `--fix` régénère `extension/strings.js`.
- Un journal développeur jamais montré à l'utilisateur peut garder un texte en dur, avec le commentaire `i18n-ok`.

## Style
- Pas de tiret cadratin (—) dans les textes. Le tiret seul « — » sert uniquement de valeur vide.
- Interface minimale : icônes plutôt que texte quand c'est clair, infobulles pour le détail, aucun texte explicatif superflu.
- Suppression : poubelle en deux clics (le premier arme, le second confirme).

## Performance
- Cap doit rester léger, mais la stabilité passe avant.
- **Ne jamais détruire une fenêtre WebView2** : ça fige le fil principal (journal du 2026-11-20). Les fenêtres sont créées à la demande puis seulement cachées.
- Pas de `TrySuspend`, de `--renderer-process-limit` ni de `--disable-gpu` : tous associés à des plantages. Toute nouvelle optimisation du moteur se valide avec `cap.log` sur une vraie machine.
- Rien de calculé chaque seconde si rien ne l'affiche.
- Ne jamais appeler l'interface (fenêtres, icône) en tenant le verrou principal depuis un autre fil.

## Journal
- `cap.log` dans le dossier de données : chaque événement important passe par `clog!` (texte en dur permis, commentaire `i18n-ok`).

## Publier
- `python scripts/bump.py X.Y.Z`, puis un commit dont le message commence par `vX.Y.Z` sur `main`.
- Tester avec de vrais clics (Playwright) : un clic simulé en JS ne donne pas le focus et masque des bogues.
