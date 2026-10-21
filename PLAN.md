# OxiNote — Plan de portage d'Obsidian en Rust

Obsidian est une application propriétaire (Electron + TypeScript, code source fermé).
« Porter » Obsidian signifie donc **réimplémenter** ses fonctionnalités en Rust,
en restant **compatible avec un vault Obsidian existant** : mêmes fichiers Markdown,
même syntaxe (wikilinks, embeds, tags, callouts…), même dossier `.obsidian/` de configuration.

## 1. Objectifs

| Priorité | Objectif |
|---|---|
| P0 | Ouvrir un vault existant sans rien casser (lecture/écriture `.md`, `.obsidian/` respecté) |
| P0 | Parser le Markdown « Obsidian-flavored » : `[[liens]]`, `![[embeds]]`, `#tags`, frontmatter YAML, `^block-id`, `==surlignage==`, `%%commentaires%%`, callouts `> [!note]`, tâches |
| P0 | Cache de métadonnées : résolution des liens, backlinks, liens non résolus, tags |
| P0 | Application desktop native : explorateur, éditeur, aperçu (lecture), backlinks, recherche, quick switcher, palette de commandes |
| P1 | Recherche plein texte avec la syntaxe Obsidian (`tag:`, `path:`, `file:`, `"phrase"`, `-exclure`, `OR`) |
| P1 | Renommage avec mise à jour automatique des liens dans tout le vault |
| P1 | Vue graphe (globale + locale) avec layout force-directed |
| P1 | Notes quotidiennes, templates (`{{date}}`, `{{time}}`, `{{title}}`) |
| P1 | CLI headless (stats, recherche, backlinks, export HTML statique) |
| P2 | Surveillance du système de fichiers (modifs externes), multi-onglets/panneaux |
| P2 | API de plugins (traits Rust, puis WASM), thèmes, Canvas (`.canvas` JSON) |

Non-objectifs (pour l'instant) : Sync/Publish (services payants), compatibilité avec les plugins JS existants.

## 2. Architecture (workspace Cargo)

```
crates/
├── oxi-core       Types partagés : NotePath, ParsedNote, Link, Tag, GraphData, traits VaultRead / LinkResolver, MemoryVault
├── oxi-markdown   Parseur Obsidian-flavored → ParsedNote ; rendu HTML ; réécriture de liens
├── oxi-vault      Vault sur disque : listing, lecture/écriture, création, corbeille, watcher, config .obsidian, daily notes, templates
├── oxi-index      MetadataCache (résolution de liens, backlinks, tags), moteur de recherche, refactor (rename + maj liens)
├── oxi-graph      Layout force-directed (Barnes-Hut), graphe local, filtres
├── oxi-cli        Binaire `oxinote` : stats, search, backlinks, export HTML
└── oxi-app        Binaire GUI `oxinote-app` (eframe/egui)
```

Graphe de dépendances :

```
oxi-core ◄── oxi-markdown ◄──┐
   ▲  ▲  ◄── oxi-vault ◄─────┤
   │  └───── oxi-graph       ├── oxi-index ◄── oxi-cli
   │                         │       ▲    ◄── oxi-app
   └─────────────────────────┘       │
```

Choix techniques :
- **Parsing** : `pulldown-cmark` (CommonMark + GFM) + pré/post-traitement maison pour la syntaxe Obsidian. Spans en octets pour permettre la réécriture exacte.
- **Frontmatter** : YAML converti en `serde_json::Value` (type neutre partagé).
- **Recherche** : index inversé maison (léger, incrémental) ; `tantivy` envisageable plus tard.
- **GUI** : `eframe`/`egui` (immédiat, natif, multiplateforme, sans Electron).
- **Watcher** : crate `notify`.
- **Résolution de liens** : règles d'Obsidian — chemin exact, puis basename unique ; en cas d'ambiguïté, le fichier le plus proche du dossier source, puis l'ordre alphabétique.

## 3. Contrat d'interfaces

Le contrat est figé dans `oxi-core` (types) et dans les **signatures publiques stubées** de chaque crate
(fonctions qui renvoient des valeurs vides par défaut). Chaque équipe implémente sa crate sans
casser ces signatures ; elle peut en **ajouter**. Ainsi tout compile dès le départ et les crates
peuvent être développées en parallèle.

## 4. Phases

| Phase | Contenu | Livrable |
|---|---|---|
| 0 | Plan, workspace, `oxi-core`, stubs des API | `cargo build` vert |
| 1 | 5 agents en parallèle (voir §5) | Chaque crate implémentée + tests |
| 2 | Intégration : merge, tests de bout en bout sur un vault exemple, correction des écarts | `cargo test --workspace` vert, app qui s'ouvre |
| 3 | P2 : plugins, thèmes, Canvas, onglets multiples, performances (vault de 10k notes) | — |

## 5. Répartition des agents (phase 1)

| Agent | Crate(s) | Responsabilités |
|---|---|---|
| A — Markdown | `oxi-markdown` | `parse`, `render_html`, `rewrite_links`, toute la syntaxe Obsidian, tests exhaustifs |
| B — Vault | `oxi-vault` | FS, config `.obsidian`, création « Untitled N », corbeille, watcher, daily notes, templates |
| C — Index | `oxi-index` | MetadataCache, résolution de liens, backlinks, tags, recherche, rename + maj des liens |
| D — Graphe & CLI | `oxi-graph`, `oxi-cli` | Layout force-directed, graphe local, CLI, export HTML statique |
| E — GUI | `oxi-app` | Application eframe : panneaux, éditeur, aperçu, recherche, switcher, palette, graphe |

Règles communes : chaque agent ne modifie **que** ses crates ; `cargo fmt`, `cargo clippy -D warnings`,
`cargo test -p <crate>` doivent passer ; pas de `unsafe`, pas de `unwrap()` hors tests.

## 5 bis. Phase 2b — Parité Obsidian (tags, propriétés, interface complète)

**Moteur (agent F)** : édition du frontmatter qui préserve le reste du fichier, propriétés typées
(`.obsidian/types.json` : text, multitext, number, checkbox, date, datetime, aliases, tags),
liste de toutes les propriétés et de leurs valeurs, arbre des tags, autocomplétion `#`,
renommage et fusion de tags dans tout le vault, commandes CLI `props`, `prop`, `tag`.

**Interface (vague 2, après la base de l'agent E)** : un agent par zone, chacun dans ses propres modules de `oxi-app` :

| Zone | Contenu (comportement Obsidian) |
|---|---|
| Éditeur | Live Preview (rendu en ligne hors ligne du curseur), autocomplétion `[[`, `#`, `![[`, raccourcis de mise en forme, cases à cocher cliquables, glisser-déposer d'images vers le dossier des pièces jointes |
| Propriétés & tags | Panneau Propriétés typé en tête de note (widgets par type, ajout/suppression/renommage, autocomplétion des valeurs), vue « Toutes les propriétés », panneau Tags hiérarchique, renommage de tag |
| Espace de travail | Onglets et panneaux scindés, explorateur complet (glisser-déposer, tri, révéler le fichier), signets, notes quotidiennes, templates, palette de commandes, raccourcis configurables, réglages (fenêtre Settings), thèmes et snippets CSS |
| Vues | Graphe global/local avec groupes de couleurs et forces réglables, recherche avec aperçu en contexte, backlinks avec contexte, plan (outline), mode lecture avec transclusion des embeds |

## 5 ter. Phase 3 — Au-delà de la parité

**Fait** : CI GitHub Actions (Linux, macOS, Windows) ; modes Live Preview / Source / Lecture
fidèles ; navigation par titres (styles « Sommaire flottant » et « Loupe ») ; lecteur PDF
(`hayro`, onglets et embeds) ; sauvegarde Git locale (commit automatique, `.gitignore`,
historique et restauration par note).

**Feuille de route** (par ordre de priorité) :

| Fonction | Contenu |
|---|---|
| Bases | Vues tableau des notes filtrées et triées par propriétés, éditables en place (format `.base` d'Obsidian) : bibliographie, suivi d'expériences, candidatures |
| Canvas | Tableau blanc de cartes, notes, images et liens (format `.canvas` / JSON Canvas) |
| Citations | Import BibTeX / Zotero, autocomplétion `[@clé]`, note de lecture par article |
| PDF (suite) | Sélection et recherche de texte, embeds en Live Preview, annotations |
| Git (suite) | Remote optionnel (push / pull) pour synchroniser entre machines |

## 6. Vault d'exemple

`examples/sample-vault/` : petit vault couvrant toutes les syntaxes, utilisé par les tests d'intégration,
la CLI et l'app.

## 7. Critères d'acceptation (fin de phase 2)

- `oxinote stats examples/sample-vault` affiche notes / liens / tags / liens non résolus corrects.
- `oxinote search examples/sample-vault 'tag:#projet "rust"'` renvoie les bons fichiers.
- `oxinote export examples/sample-vault out/` produit un site HTML navigable (wikilinks → liens relatifs).
- L'app s'ouvre sur un vault, édite et sauvegarde une note, affiche les backlinks et le graphe.
- Renommer une note met à jour tous les liens entrants.
