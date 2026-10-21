# Guide pour les agents — OxiNote

À lire en entier avant de coder. Il évite de ré-explorer le dépôt.

## Le projet
Réimplémentation d'Obsidian en Rust, compatible avec les vaults existants. Workspace Cargo :

| Crate | Rôle | État |
|---|---|---|
| `oxi-core` | Types partagés : `NotePath`, `ParsedNote`, `Link`, `Tag`, `GraphData`, traits `VaultRead`/`VaultWrite`/`LinkResolver`, `MemoryVault` | stable, ne pas modifier |
| `oxi-markdown` | `parse`, `render_html`, `rewrite_links`, `slugify`, module `properties` (édition du frontmatter qui préserve le reste du fichier : `set_property`, `remove_property`, `rename_property`, `ordered_properties`, `add_tag`/`remove_tag`/`rename_tag`, `add_alias`/`remove_alias`, `frontmatter_status`) | complet |
| `oxi-vault` | `Vault` sur disque, `AppConfig` (`.obsidian/app.json`), daily notes, templates (`expand_template`, `format_moment`), watcher, `PropertyType` + `Vault::property_types()`/`set_property_type()` (`types.json`) | complet |
| `oxi-index` | `MetadataCache` (résolution, backlinks, tags, `tag_tree`, `tag_suggestions`, `properties`, `property_values`), `SearchIndex` (syntaxe Obsidian), `fuzzy_find(_with)`, `rename_with_links`, `rename_folder_with_links`, `rename_tag`, `set_property_in`… | complet |
| `oxi-graph` | `ForceLayout` (Barnes-Hut, alpha/reheat), `local_graph`, `filter_graph`, `color_groups`, `neighbors` | complet |
| `oxi-cli` | binaire `oxinote` | complet |
| `oxi-app` | binaire GUI `oxinote-app` (eframe/egui 0.3x) | en cours |

Lis la rustdoc d'une API **avant** de l'utiliser (`grep -n "pub fn" crates/<crate>/src/*.rs`) : ne devine pas les signatures.

## Architecture de `oxi-app` (`crates/oxi-app/src/`)
- `main.rs` : parsing des arguments, dont les flags cachés de capture d'écran (`--screenshot FILE --frames N --size WxH --open NOTE --reading --graph --palette --switcher Q --search Q --theme dark|light --left files|search|tags --right backlinks|outgoing|outline --demo --fresh`).
- `backend.rs` : **deux traits**, `VaultBackend: VaultWrite` (implémenté par `DiskVault` et `MemVault`) et `IndexBackend: LinkResolver` (implémenté par `RealIndex`, plus le faux index de `fake.rs`). **Toute nouvelle méthode de trait doit être implémentée dans les deux implémentations.** Le faux index peut renvoyer une valeur vide raisonnable.
- `state.rs` : `AppState` (vault, index, commands, settings, files, folders, tabs, active, buffers `HashMap<NotePath, Buffer>`, modal, toasts, generation, search…). Toute la logique métier passe par là et se teste sans écran. `Tab { id, content: TabContent }`, `NoteTab { path, mode: ViewMode, back, forward, pending_line }`, `GraphTab`. `Buffer { text, saved, last_edit, conflict }`, avec `is_dirty()`.
- `ui/mod.rs` : `OxinoteApp` (eframe::App), `enum Action` + `apply()` (l'UI émet des actions, `apply` les exécute sur `AppState`), `ribbon`, `view_header`, `central`, `workspace`, `toasts`.
- `ui/editor.rs` (édition source + layouter), `ui/panes.rs` (explorateur, recherche, tags, backlinks, outgoing, outline), `ui/graph.rs`, `ui/modals.rs` (quick switcher, palette, confirmations), `ui/theme.rs`, `ui/tests.rs` (tests egui headless).
- `reading.rs` : modèle et rendu du mode lecture. `highlight.rs` : coloration source.
- `commands.rs` : `enum Command` + `CommandRegistry` (id, nom, raccourci) ; la palette et les raccourcis partagent ce registre.
- `settings.rs` : `Settings` (thème, sidebars `LeftPane`/`RightPane`, tailles de police…), persistés via eframe storage.

## Règles (obligatoires)
1. Tu travailles **uniquement** dans le worktree qu'on t'indique (chemin absolu). Fais `cd` dedans avant toute commande. Ne touche jamais `/home/user/OxiNote` lui-même ni les autres worktrees.
2. Ne modifie que ta zone. Mets le nouveau code dans de **nouveaux modules**. Dans les fichiers partagés (`state.rs`, `ui/mod.rs`, `backend.rs`, `fake.rs`, `commands.rs`, `settings.rs`, `main.rs`), fais des ajouts **minimaux** (nouveaux variants, champs, méthodes, un appel), sans reformater ni réorganiser.
3. Pas de `unsafe`, pas de `unwrap()`/`expect()` hors tests, pas de panique sur les erreurs d'I/O (affiche un toast avec `state.error(..)`).
4. La logique doit vivre dans des fonctions pures ou dans `AppState`, avec des tests unitaires. L'UI egui reste fine.
5. Ne jamais écrire dans `examples/sample-vault/`. Pour les tests qui écrivent, copie le vault dans un dossier temporaire (`tempfile`).
6. Vérification visuelle :
   `xvfb-run -a -s "-screen 0 1400x900x24" cargo run -q -p oxi-app -- examples/sample-vault --screenshot /tmp/x.png --frames 30 --size 1280x800 [flags]`,
   puis ouvre le PNG avec l'outil Read. Si `libxkbcommon-x11` manque : `apt-get install -y libxkbcommon-x11-0`. Compare avec l'apparence d'Obsidian et corrige.
7. Avant de finir, tout doit passer : `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
8. Commit dans ton worktree (sans push), avec un message qui se termine par :
   ```
   Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
   Claude-Session: https://claude.ai/code/session_01Yc5gx916Y8nW8UpkKgs6bn
   ```
   (Garde exactement ces lignes.)
9. Si une étape bloque plus de ~3 essais, simplifie (fonctionnalité réduite mais propre) et note-le dans ton rapport. Ne laisse jamais le code en état non compilable.
10. Rapport final (court) : branche et commit, ce qui marche, ce qui manque, fichiers partagés modifiés (pour la fusion), chemins des captures.

## Économie de tokens
- Ne relis pas des fichiers entiers inutilement : utilise `grep -n` et `sed -n 'a,bp'`.
- `cargo check -p oxi-app` pour itérer ; `cargo test -p oxi-app` ; le workspace complet uniquement à la fin.
- Captures : 2 ou 3 au maximum par itération visuelle.
