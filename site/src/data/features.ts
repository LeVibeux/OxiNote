export type Lang = "fr" | "en";

export const GITHUB = "https://github.com/LeVibeux/OxiNote";
export const RELEASES = "https://github.com/LeVibeux/OxiNote/releases";

export const ui = {
  fr: {
    skip: "Aller au contenu",
    menu: "Menu",
    langLabel: "Langue",
    navLabel: "Pages",
    footerLabel: "Pied de page",
    github: "GitHub",
    contribute: "Pour contribuer",
    download: "Télécharger",
    privacy: "Mentions légales",
    publisher: "Éditeur",
    fontsLead: "Polices",
    footNote: "Pas de témoin. Pas de mesure d’audience.",
    next: "Suite",
    tech: "Détail",
    planned: "prévu",
    releasesNote: "Chaque bouton ouvre la page GitHub Releases.",
    sessionCaption:
      "Sortie réelle d’oxinote 0.3.0, lancée sur examples/sample-vault. Les lignes de commande sont les invocations ; le texte qui suit est la sortie du programme, sans retouche.",
    helpCaption: "oxinote --help",
    graphCaption: "oxinote graph examples/sample-vault --hide-orphans",
    tagCaption: "oxinote tag --help",
    propCaption: "oxinote prop --help",
    notFoundTitle: "Page absente",
    notFoundText: "Cette adresse ne correspond à aucune page.",
    home: "Accueil",
  },
  en: {
    skip: "Skip to content",
    menu: "Menu",
    langLabel: "Language",
    navLabel: "Pages",
    footerLabel: "Footer",
    github: "GitHub",
    contribute: "Contribute",
    download: "Download",
    privacy: "Privacy",
    publisher: "Publisher",
    fontsLead: "Typefaces",
    footNote: "No cookies. No audience measurement.",
    next: "Next",
    tech: "Detail",
    planned: "planned",
    releasesNote: "Each button opens the GitHub Releases page.",
    sessionCaption:
      "Real output of oxinote 0.3.0, run on examples/sample-vault. The command lines are the invocations; the text under each one is the program’s output, unedited.",
    helpCaption: "oxinote --help",
    graphCaption: "oxinote graph examples/sample-vault --hide-orphans",
    tagCaption: "oxinote tag --help",
    propCaption: "oxinote prop --help",
    notFoundTitle: "No such page",
    notFoundText: "This address does not match a page.",
    home: "Home",
  },
} as const;

export const downloads = [
  { id: "deb", fr: "Linux .deb", en: "Linux .deb", note: { fr: "Ubuntu, Debian", en: "Ubuntu, Debian" } },
  { id: "arm", fr: "macOS Apple Silicon", en: "macOS Apple Silicon", note: { fr: "M1 et suivants", en: "M1 and later" } },
  { id: "intel", fr: "macOS Intel", en: "macOS Intel", note: { fr: "x86_64", en: "x86_64" } },
  { id: "win", fr: "Windows", en: "Windows", note: { fr: "x86_64", en: "x86_64" } },
] as const;

export interface Shot {
  src: string;
  alt: Record<Lang, string>;
  caption: Record<Lang, string>;
}

export interface Feature {
  id: string;
  slug: Record<Lang, string>;
  nav: Record<Lang, string>;
  blurb: Record<Lang, string>;
  title: Record<Lang, string>;
  description: Record<Lang, string>;
  lede: Record<Lang, string>;
  body: Record<Lang, string[]>;
  tech: Record<Lang, string[]>;
  shots: Shot[];
  demo?: "modes" | "pan" | "term" | "downloads";
}

const liveAlt = {
  fr: "Thème clair : explorateur du vault sample-vault (attachments, Journal, Projets, Templates, Accueil, Idées, Orpheline), note Idées avec titres, cases à cocher, surlignage et callout, mentions liées à droite.",
  en: "Light theme: sample-vault file tree (attachments, Journal, Projets, Templates, Accueil, Idées, Orpheline), the Idées note with headings, checkboxes, highlight and a callout, linked mentions on the right.",
};

export const features: Feature[] = [
  {
    id: "vault",
    slug: { fr: "coffre", en: "vault" },
    nav: { fr: "Coffre", en: "Vault" },
    blurb: {
      fr: "Un dossier Obsidian déjà là, ouvert sans le convertir.",
      en: "An Obsidian folder that is already yours, opened without converting it.",
    },
    title: {
      fr: "Le coffre s’ouvre tel quel.",
      en: "The vault opens as it is.",
    },
    description: {
      fr: "OxiNote ouvre un dossier de vault Obsidian. Les notes restent des fichiers Markdown.",
      en: "OxiNote opens an Obsidian vault folder. Notes stay Markdown files.",
    },
    lede: {
      fr: "On désigne le dossier. Les notes, les pièces jointes et le dossier `.obsidian/` sont déjà les vôtres.",
      en: "Point it at the folder. The notes, the attachments and the `.obsidian/` folder are already yours.",
    },
    body: {
      fr: [
        "OxiNote ne crée pas un autre format. Une note est un fichier `.md` dans le dossier du vault. Les liens `[[…]]`, les embeds et les étiquettes sont ceux du fichier.",
        "Le dossier `.obsidian/` est lu et réécrit sur place : réglages, plugins de cœur, signets, notes quotidiennes, modèles, types de propriétés. Les clés inconnues d’`app.json` sont conservées à l’écriture.",
      ],
      en: [
        "OxiNote does not invent another format. A note is a `.md` file in the vault folder. Wikilinks, embeds and tags are the ones in the file.",
        "The `.obsidian/` folder is read and written in place: settings, core plugins, bookmarks, daily notes, templates, property types. Unknown keys in `app.json` are kept when that file is written.",
      ],
    },
    tech: {
      fr: [
        "Neuf plugins de cœur ont un interrupteur, fichier `.obsidian/core-plugins.json` : `backlink`, `bookmarks`, `daily-notes`, `graph`, `outline`, `properties`, `tag-pane`, `templates`, `word-count`. L’ancienne forme (liste d’identifiants) et la forme objet sont lues. L’écriture se fait en objet. Un identifiant inconnu est conservé.",
        "La capture montre le vault d’exemple en thème clair. Le bandeau d’état indique Editing.",
      ],
      en: [
        "Nine core plugins have a toggle, in `.obsidian/core-plugins.json`: `backlink`, `bookmarks`, `daily-notes`, `graph`, `outline`, `properties`, `tag-pane`, `templates`, `word-count`. Both the older list of ids and the object form are read. Writing uses the object form. An unknown id is kept.",
        "The capture shows the sample vault in the light theme. The status bar reads Editing.",
      ],
    },
    shots: [
      {
        src: "/media/live-preview-light.png",
        alt: liveAlt,
        caption: {
          fr: "Vault d’exemple, thème clair. Fichier du dépôt : `crates/oxi-app/screenshots/live-preview-light.png`.",
          en: "Sample vault, light theme. Repository file: `crates/oxi-app/screenshots/live-preview-light.png`.",
        },
      },
    ],
  },
  {
    id: "modes",
    slug: { fr: "modes", en: "modes" },
    nav: { fr: "Modes", en: "Modes" },
    blurb: {
      fr: "Live Preview, Source et Lecture, sur le même fichier.",
      en: "Live Preview, Source and Reading, on the same file.",
    },
    title: {
      fr: "Trois façons de tenir la même note.",
      en: "Three ways to hold the same note.",
    },
    description: {
      fr: "Live Preview, Source et Lecture. Le fichier Markdown ne change pas de format.",
      en: "Live Preview, Source and Reading. The Markdown file does not change format.",
    },
    lede: {
      fr: "Live Preview habille le Markdown sans le sortir du tampon. Source le montre brut. Lecture le rend pour la lecture.",
      en: "Live Preview dresses the Markdown without taking it out of the buffer. Source shows it raw. Reading renders it for reading.",
    },
    body: {
      fr: [
        "En Live Preview, le texte source reste dans le tampon, donc le curseur et la sélection tombent sur les vrais caractères. Les marqueurs des lignes qui n’ont pas le curseur sont dessinés tout petits et transparents. Les puces, les cases et les filets sont peints par-dessus.",
        "Source affiche le Markdown. Lecture affiche la note rendue. Le bandeau d’état de ces captures nomme les trois : Live Preview, Source mode, Reading view.",
      ],
      en: [
        "In Live Preview the source stays in the buffer, so the caret and the selection land on the real characters. Markers on lines that do not hold the caret are drawn tiny and transparent. Bullets, checkboxes and rules are painted over them.",
        "Source shows the Markdown. Reading shows the rendered note. The status bar in these captures names the three: Live Preview, Source mode, Reading view.",
      ],
    },
    tech: {
      fr: [
        "La bascule est dans l’en-tête de la note. Le fondu ci-dessus alterne trois captures du même écran : `modes-live.png`, `modes-source.png`, `modes-reading.png`. Sans mouvement, les trois images restent l’une sous l’autre.",
      ],
      en: [
        "The switch sits in the note header. The fade above alternates three captures of the same screen: `modes-live.png`, `modes-source.png`, `modes-reading.png`. Without motion, the three images stay stacked.",
      ],
    },
    demo: "modes",
    shots: [
      {
        src: "/media/modes-live.png",
        alt: {
          fr: "Live Preview, thème sombre : note Modes demo, propriétés, code, tableau, callouts et listes. Le bandeau indique Live Preview.",
          en: "Live Preview, dark theme: Modes demo note, properties, code, table, callouts and lists. The status bar reads Live Preview.",
        },
        caption: {
          fr: "Live Preview.",
          en: "Live Preview.",
        },
      },
      {
        src: "/media/modes-source.png",
        alt: {
          fr: "Source mode : le Markdown brut de la même note, frontmatter, dièses, marqueurs de gras et de surlignage. Le bandeau indique Source mode.",
          en: "Source mode: the raw Markdown of the same note, frontmatter, hashes, bold and highlight markers. The status bar reads Source mode.",
        },
        caption: {
          fr: "Source.",
          en: "Source.",
        },
      },
      {
        src: "/media/modes-reading.png",
        alt: {
          fr: "Reading view : la même note rendue, callout d’avertissement replié, sans marqueurs. Le bandeau indique Reading view.",
          en: "Reading view: the same note rendered, warning callout folded, markers hidden. The status bar reads Reading view.",
        },
        caption: {
          fr: "Lecture.",
          en: "Reading.",
        },
      },
    ],
  },
  {
    id: "graph",
    slug: { fr: "graphe", en: "graph" },
    nav: { fr: "Graphe", en: "Graph" },
    blurb: {
      fr: "Les liens entre les notes, et ce qui pointe vers celle qui est ouverte.",
      en: "The links between notes, and what points at the one that is open.",
    },
    title: {
      fr: "Voir qui relie quoi.",
      en: "See what links where.",
    },
    description: {
      fr: "Le graphe dessine les notes et leurs liens. Le panneau latéral liste les mentions liées.",
      en: "The graph draws notes and their links. The side panel lists linked mentions.",
    },
    lede: {
      fr: "Le graphe pose les notes et tire les liens. À côté de la note ouverte, les mentions liées montrent d’où elle est citée.",
      en: "The graph places the notes and draws the links. Beside the open note, linked mentions show where it is cited.",
    },
    body: {
      fr: [
        "La disposition écarte les nœuds les uns des autres avec une répulsion de Barnes-Hut, et les liens les rapprochent. Un graphe local peut se limiter à la note ouverte.",
        "Le panneau de la seconde capture s’intitule Linked mentions. Le bandeau compte les backlinks. La commande `oxinote backlinks` liste les mêmes liens depuis le terminal.",
      ],
      en: [
        "The layout pushes nodes apart with Barnes-Hut repulsion, and the links pull them together. A local graph can stay around the open note.",
        "The panel in the second capture is titled Linked mentions. The status bar counts backlinks. The `oxinote backlinks` command lists the same kind of links from the terminal.",
      ],
    },
    tech: {
      fr: [
        "Sur la capture du graphe, un nœud s’intitule Canvas. C’est un titre de note dans cette capture, pas le tableau blanc Canvas, qui n’est pas dans l’application.",
        "Filtres visibles sur la capture : étiquettes, pièces jointes, fichiers existants, orphelines, groupes, forces (centre, répulsion, lien, distance).",
      ],
      en: [
        "On the graph capture, one node is titled Canvas. That is a note title in this capture, not the Canvas whiteboard, which is not in the application.",
        "Filters visible on the capture: tags, attachments, existing files, orphans, groups, forces (center, repulsion, link, distance).",
      ],
    },
    shots: [
      {
        src: "/media/graph.png",
        alt: {
          fr: "Graphe sombre : nœuds Accueil, Idées, ObsiRust, Rust notes, 2026-10-20, Note fantôme et Canvas, avec le panneau de filtres et de forces.",
          en: "Dark graph: nodes Accueil, Idées, ObsiRust, Rust notes, 2026-10-20, Note fantôme and Canvas, with the filter and forces panel.",
        },
        caption: {
          fr: "Graphe. Fichier : `screenshots/graph.png`. Le nœud Canvas est un titre de cette capture.",
          en: "Graph. File: `screenshots/graph.png`. The Canvas node is a title in this capture.",
        },
      },
      {
        src: "/media/backlinks.png",
        alt: {
          fr: "Note Accueil et panneau Linked mentions : extraits depuis Idées et ObsiRust, bandeau à 3 backlinks.",
          en: "Accueil note and the Linked mentions panel: excerpts from Idées and ObsiRust, status bar at 3 backlinks.",
        },
        caption: {
          fr: "Mentions liées. Fichier : `screenshots/backlinks.png`.",
          en: "Linked mentions. File: `screenshots/backlinks.png`.",
        },
      },
    ],
  },
  {
    id: "git",
    slug: { fr: "historique", en: "history" },
    nav: { fr: "Historique", en: "History" },
    blurb: {
      fr: "Un Git local, sur la machine, sans dépôt distant.",
      en: "Local Git, on the machine, with no remote.",
    },
    title: {
      fr: "L’historique reste sur la machine.",
      en: "History stays on the machine.",
    },
    description: {
      fr: "OxiNote versionne le vault avec le Git du système. Il n’y a pas de dépôt distant.",
      en: "OxiNote versions the vault with the system Git. There is no remote.",
    },
    lede: {
      fr: "Le programme `git` de la machine enregistre le vault. Les commits ne partent nulle part : pas de push, pas de pull.",
      en: "The machine’s `git` program records the vault. Commits go nowhere else: no push, no pull.",
    },
    body: {
      fr: [
        "L’initialisation crée un dépôt local, un `.gitignore` et un premier commit. La sauvegarde automatique fait un commit à intervalle régulier s’il y a des modifications. Une autre option fait un dernier commit en quittant. Le message par défaut est `vault backup: {{date}}`.",
        "L’historique d’une note liste les commits qui la touchent. Restaurer une version remplace le texte de la note. Le texte précédent reste dans l’historique s’il a été commité.",
      ],
      en: [
        "Initialising creates a local repository, a `.gitignore` and a first commit. Automatic backup makes a commit at a regular interval when there are changes. Another option makes a last commit on quit. The default message is `vault backup: {{date}}`.",
        "A note’s history lists the commits that touch it. Restoring a version replaces the note’s text. The previous text stays in the history if it was committed.",
      ],
    },
    tech: {
      fr: [
        "Tout passe par le binaire `git` du système, dans un fil à part. L’interface ne se bloque pas dessus. Les réglages du vault sont dans `.obsidian/oxinote-git.json`.",
        "Quatre cases de `.gitignore` : ignorer la configuration `.obsidian`, les pièces jointes lourdes (`*.pdf`, `*.mp4`, `*.zip`), la corbeille, l’état de l’espace de travail.",
        "Le défilement de la première image est une animation sur la capture `git-history.png`. Sans mouvement, l’image est entière.",
      ],
      en: [
        "Everything goes through the system `git` binary, on a side thread. The interface does not block on it. Per-vault settings live in `.obsidian/oxinote-git.json`.",
        "Four `.gitignore` toggles: ignore `.obsidian` configuration, heavy attachments (`*.pdf`, `*.mp4`, `*.zip`), the trash, and the workspace state.",
        "The pan on the first image is an animation of the capture `git-history.png`. Without motion, the whole image is shown.",
      ],
    },
    demo: "pan",
    shots: [
      {
        src: "/media/git-history.png",
        alt: {
          fr: "Historique d’Accueil : trois commits vault backup datés, comparaison avec la version actuelle, ligne ajoutée en vert, bouton Restaurer cette version.",
          en: "History of Accueil: three dated vault backup commits, comparison with the current version, an added line in green, Restore this version button.",
        },
        caption: {
          fr: "Historique d’une note. Fichier : `crates/oxi-app/screenshots/git-history.png`.",
          en: "History of one note. File: `crates/oxi-app/screenshots/git-history.png`.",
        },
      },
      {
        src: "/media/git-settings.png",
        alt: {
          fr: "Fenêtre Sauvegarde (Git) : sauvegarde automatique, intervalle en minutes, sauvegarde à la fermeture, message vault backup: {{date}}, cases gitignore.",
          en: "Backup (Git) window: automatic backup, interval in minutes, backup on quit, message vault backup: {{date}}, gitignore toggles.",
        },
        caption: {
          fr: "Réglages. Fichier : `crates/oxi-app/screenshots/git-settings.png`.",
          en: "Settings. File: `crates/oxi-app/screenshots/git-settings.png`.",
        },
      },
    ],
  },
  {
    id: "pdf",
    slug: { fr: "pdf", en: "pdf" },
    nav: { fr: "PDF", en: "PDF" },
    blurb: {
      fr: "Un lecteur de PDF dans un onglet. Pas un export PDF.",
      en: "A PDF reader in a tab. Not a PDF export.",
    },
    title: {
      fr: "Lire un PDF dans un onglet.",
      en: "Read a PDF in a tab.",
    },
    description: {
      fr: "OxiNote affiche un PDF avec hayro. Il n’exporte pas les notes en PDF.",
      en: "OxiNote displays a PDF with hayro. It does not export notes to PDF.",
    },
    lede: {
      fr: "Un lien ou un embed ouvre le PDF dans un onglet, à la page et au zoom demandés. Ce n’est pas une exportation : la commande `export` produit un site HTML.",
      en: "A link or an embed opens the PDF in a tab, at the page and zoom asked for. This is not an export: the `export` command produces an HTML site.",
    },
    body: {
      fr: [
        "Le moteur est hayro, en Rust, sans bibliothèque système. Un fil à part ouvre le document et dessine les pages, pour ne pas bloquer l’interface.",
        "La barre de l’onglet, sur la capture, indique la page, le zoom, Fit to width, et Open in default app.",
      ],
      en: [
        "The engine is hayro, in Rust, with no system library. A side thread opens the document and draws the pages, so the interface does not block.",
        "The tab bar in the capture shows the page, the zoom, Fit to width, and Open in default app.",
      ],
    },
    tech: {
      fr: [
        "Un lien peut viser une page et une hauteur, sous la forme `#page=3` et `height=400`, ou `|400`. L’embed de la seconde capture ouvre la page 2.",
        "Il n’y a pas d’export PDF des notes. `oxinote export` écrit un dossier de pages HTML.",
      ],
      en: [
        "A link can name a page and a height, as `#page=3` and `height=400`, or `|400`. The embed in the second capture opens page 2.",
        "There is no PDF export of notes. `oxinote export` writes a folder of HTML pages.",
      ],
    },
    shots: [
      {
        src: "/media/pdf-tab.png",
        alt: {
          fr: "Onglet PDF test.pdf, page 1 sur 3, zoom 82 %, boutons Fit to width et Open in default app. Le panneau Linked mentions liste des liens vers ce PDF.",
          en: "PDF tab test.pdf, page 1 of 3, zoom 82%, Fit to width and Open in default app buttons. The Linked mentions panel lists links to this PDF.",
        },
        caption: {
          fr: "Lecteur en onglet. Fichier : `crates/oxi-app/screenshots/pdf-tab.png`.",
          en: "Reader in a tab. File: `crates/oxi-app/screenshots/pdf-tab.png`.",
        },
      },
      {
        src: "/media/pdf-embed.png",
        alt: {
          fr: "Note en lecture avec un PDF embarqué à la page 2, info-bulle Open test.pdf at page 2.",
          en: "Reading note with an embedded PDF on page 2, tooltip Open test.pdf at page 2.",
        },
        caption: {
          fr: "Embed. Fichier : `crates/oxi-app/screenshots/pdf-embed.png`.",
          en: "Embed. File: `crates/oxi-app/screenshots/pdf-embed.png`.",
        },
      },
    ],
  },
  {
    id: "cli",
    slug: { fr: "commande", en: "command" },
    nav: { fr: "Commande", en: "Command" },
    blurb: {
      fr: "La commande oxinote interroge le vault sans ouvrir la fenêtre.",
      en: "The oxinote command queries the vault without opening the window.",
    },
    title: {
      fr: "Le coffre, en ligne de commande.",
      en: "The vault, from the command line.",
    },
    description: {
      fr: "oxinote lit un vault Obsidian : statistiques, recherche, liens, étiquettes, export HTML.",
      en: "oxinote reads an Obsidian vault: statistics, search, links, tags, HTML export.",
    },
    lede: {
      fr: "Chaque sous-commande prend le dossier du vault en premier argument. `export` écrit un site HTML statique. Ce n’est pas un export PDF.",
      en: "Every subcommand takes the vault folder as its first argument. `export` writes a static HTML site. It is not a PDF export.",
    },
    body: {
      fr: [
        "La recherche accepte la syntaxe `tag:`, `path:`, `file:`, une phrase entre guillemets, `-mot`, `OR` et `/regex/`. `rename` et `tag rename` ont un `--dry-run` qui n’écrit rien. `daily` affiche le chemin de la note du jour, et la crée si elle manque.",
        "Dans l’export HTML, un wikilink devient un lien relatif vers la page générée. Un lien vers une note absente reste marqué non résolu.",
      ],
      en: [
        "Search accepts `tag:`, `path:`, `file:`, a quoted phrase, `-word`, `OR` and `/regex/`. `rename` and `tag rename` have a `--dry-run` that writes nothing. `daily` prints the path of the daily note, and creates it if it is missing.",
        "In the HTML export, a wikilink becomes a relative link to the generated page. A link to a missing note stays marked unresolved.",
      ],
    },
    tech: {
      fr: [
        "Sous-commandes, d’après `oxinote --help` : stats, search, backlinks, links, unresolved, tags, props, prop, tag, orphans, find, graph, rename, daily, export.",
        "`graph` écrit du JSON (nœuds et arêtes) ou du Graphviz. `prop set` et `prop rm` modifient le frontmatter. `tag` renomme, ajoute ou retire une étiquette.",
        "Le terminal plus bas rejoue une session réelle sur `examples/sample-vault`, version 0.3.0. La phrase « 1 attachments » est celle du programme.",
      ],
      en: [
        "Subcommands, from `oxinote --help`: stats, search, backlinks, links, unresolved, tags, props, prop, tag, orphans, find, graph, rename, daily, export.",
        "`graph` writes JSON (nodes and edges) or Graphviz. `prop set` and `prop rm` edit frontmatter. `tag` renames, adds or removes a tag.",
        "The terminal below replays a real session on `examples/sample-vault`, version 0.3.0. The phrase “1 attachments” is the program’s own.",
      ],
    },
    demo: "term",
    shots: [],
  },
  {
    id: "install",
    slug: { fr: "installation", en: "install" },
    nav: { fr: "Installation", en: "Install" },
    blurb: {
      fr: "Les fichiers de la page Releases, contrôlés par SHA-256.",
      en: "The files on the Releases page, checked with SHA-256.",
    },
    title: {
      fr: "Installer, et vérifier la somme.",
      en: "Install, and check the sum.",
    },
    description: {
      fr: "Linux .deb, macOS Apple Silicon, macOS Intel, Windows. Les mises à jour comparent l’archive à SHA256SUMS.txt.",
      en: "Linux .deb, macOS Apple Silicon, macOS Intel, Windows. Updates compare the archive to SHA256SUMS.txt.",
    },
    lede: {
      fr: "Les publications GitHub contiennent l’application et la commande `oxinote`. Au démarrage, une mise à jour éventuelle est comparée à `SHA256SUMS.txt` avant d’être installée.",
      en: "The GitHub releases contain the application and the `oxinote` command. On startup, a possible update is compared to `SHA256SUMS.txt` before it is installed.",
    },
    body: {
      fr: [
        "La page Releases propose un `.deb` pour Ubuntu et Debian, une archive macOS Apple Silicon (`aarch64-apple-darwin`), une archive macOS Intel (`x86_64-apple-darwin`), une archive Windows (`x86_64-pc-windows-msvc`), et une archive Linux `x86_64-unknown-linux-gnu`. Les sommes sont dans `SHA256SUMS.txt`.",
        "L’application interroge au plus une fois par jour l’API `releases/latest`. Si le tag est plus récent, elle télécharge l’archive de la plateforme, la compare à `SHA256SUMS.txt` et remplace les fichiers installés. La version nouvelle s’exécute au redémarrage.",
      ],
      en: [
        "The Releases page offers a `.deb` for Ubuntu and Debian, a macOS Apple Silicon archive (`aarch64-apple-darwin`), a macOS Intel archive (`x86_64-apple-darwin`), a Windows archive (`x86_64-pc-windows-msvc`), and a Linux `x86_64-unknown-linux-gnu` archive. The sums are in `SHA256SUMS.txt`.",
        "The application asks the `releases/latest` API at most once a day. If the tag is newer, it downloads the archive for the platform, compares it to `SHA256SUMS.txt` and replaces the installed files. The new version runs after a restart.",
      ],
    },
    tech: {
      fr: [
        "Réglages, About : « Installer automatiquement » (valeur par défaut du code), « Me prévenir seulement », ou « Désactivées ».",
        "Une installation que le processus ne peut pas réécrire est seulement signalée : le paquet `.deb` sous `/usr`, un dossier en lecture seule, une application macOS transloquée.",
        "L’archive mise à jour se nomme `oxinote-<tag>-<cible>.tar.gz` sous Linux et `.zip` ailleurs. Le `.deb` n’est pas le fichier que cette mise à jour remplace tout seul.",
      ],
      en: [
        "Settings, About: “Installer automatiquement” (the code’s default), “Me prévenir seulement”, or “Désactivées”.",
        "An install the process cannot rewrite is only reported: the `.deb` package under `/usr`, a read-only folder, a translocated macOS app.",
        "The update archive is named `oxinote-<tag>-<target>.tar.gz` on Linux and `.zip` elsewhere. The `.deb` is not the file that this update replaces on its own.",
      ],
    },
    demo: "downloads",
    shots: [],
  },
  {
    id: "later",
    slug: { fr: "pas-encore", en: "not-yet" },
    nav: { fr: "Pas encore", en: "Not yet" },
    blurb: {
      fr: "Bases, Canvas, citations, Git distant : prévus, pas livrés.",
      en: "Bases, Canvas, citations, remote Git: planned, not shipped.",
    },
    title: {
      fr: "Pas encore là.",
      en: "Not there yet.",
    },
    description: {
      fr: "Bases, Canvas, les citations et le Git distant sont prévus. Ils ne sont pas dans la version 0.3.0.",
      en: "Bases, Canvas, citations and remote Git are planned. They are not in version 0.3.0.",
    },
    lede: {
      fr: "Quatre choses sont prévues. Elles ne sont pas dans l’application aujourd’hui. Le lecteur PDF et le Git local, eux, y sont.",
      en: "Four things are planned. They are not in the application today. The PDF reader and local Git are.",
    },
    body: {
      fr: [
        "Bases — vues tableau des notes, filtrées et triées par propriétés, éditables, au format `.base`.",
        "Canvas — tableau blanc de cartes, de notes, d’images et de liens, au format `.canvas`. Un fichier peut porter ce suffixe dans le dossier ; ce n’est pas l’éditeur de tableau blanc.",
        "Citations — import BibTeX ou Zotero, autocomplétion `[@clé]`, note de lecture.",
        "Git distant — push et pull vers un dépôt. La sauvegarde locale, sans distant, est déjà là.",
      ],
      en: [
        "Bases — table views of notes, filtered and sorted by properties, editable, in the `.base` format.",
        "Canvas — a whiteboard of cards, notes, images and links, in the `.canvas` format. A file in the folder can wear that suffix; that is not the whiteboard editor.",
        "Citations — BibTeX or Zotero import, `[@key]` completion, a reading note.",
        "Remote Git — push and pull to a repository. Local backup, with no remote, is already there.",
      ],
    },
    tech: {
      fr: [
        "Le mot « prévu » sur cette page veut dire : nommé dans la feuille de route, absent de l’application 0.3.0. Rien ici n’est une date.",
      ],
      en: [
        "The word “planned” on this page means: named on the roadmap, absent from the 0.3.0 application. Nothing here is a date.",
      ],
    },
    shots: [],
  },
];

export function otherLang(lang: Lang): Lang {
  return lang === "fr" ? "en" : "fr";
}

export function featurePath(feature: Feature, lang: Lang): string {
  return `/${lang}/${feature.slug[lang]}/`;
}

export function pathFor(lang: Lang, id: string | "home" | "privacy"): string {
  if (id === "home") return `/${lang}/`;
  if (id === "privacy") return lang === "fr" ? "/fr/confidentialite/" : "/en/privacy/";
  const feature = features.find((item) => item.id === id);
  if (!feature) return `/${lang}/`;
  return featurePath(feature, lang);
}
