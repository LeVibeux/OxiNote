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
  demo?: "modes" | "pan" | "term" | "downloads" | "nav";
}

const liveAlt = {
  fr: "Thème clair : explorateur (Idées, Lecture, page.pdf), note Idées avec une liste, un callout et un surlignage, mention liée à droite.",
  en: "Light theme: file tree (Idées, Lecture, page.pdf), the Idées note with a list, a callout and a highlight, a linked mention on the right.",
};

export const features: Feature[] = [
  {
    id: "nav",
    slug: { fr: "navigation", en: "navigation" },
    nav: { fr: "Navigation", en: "Navigation" },
    blurb: {
      fr: "Un tiret par titre, sur le bord de la note.",
      en: "One tick per heading, on the edge of the note.",
    },
    title: {
      fr: "Sache toujours où tu en es.",
      en: "Always know where you are.",
    },
    description: {
      fr: "Sur le bord de chaque note, un tiret par titre. Celui que tu lis s’allume.",
      en: "One tick per heading on the edge of every note. The one you’re reading lights up.",
    },
    lede: {
      fr: "Sur le bord de chaque note, un tiret par titre. Celui que tu lis s’allume. Survole pour voir le plan, clique pour y sauter.",
      en: "One tick per heading on the edge of every note. The one you’re reading lights up. Hover to see the outline, click to jump.",
    },
    body: {
      fr: [
        "Les tirets sont empilés sur le bord droit. Un titre de niveau 1 fait 20 px, un titre de niveau 2 fait 14 px, un titre de niveau 3 ou plus fait 8 px.",
        "Celui du titre visible prend la couleur d’accent. C’est le dernier titre dont le haut est passé au-dessus du bord visible, avec une petite marge. Il suit le défilement, pas le curseur.",
      ],
      en: [
        "The ticks stack on the right edge. A level-1 heading is 20 px, a level-2 heading is 14 px, a level-3 heading or deeper is 8 px.",
        "The tick of the heading in view takes the accent colour. That is the last heading whose top has passed above the visible edge, with a small margin. It follows scrolling, not the caret.",
      ],
    },
    tech: {
      fr: [
        "Loupe, le réglage par défaut : les tirets près du pointeur grandissent, à la façon du Dock, et leur titre s’écrit dans une pastille.",
        "Plan flottant : une carte liste le plan, indenté. Le titre en cours est en gras et dans la couleur d’accent. Un titre de niveau 1 l’est aussi.",
        "Un clic sur un tiret, ou sur une ligne de la carte, saute à ce titre. Dans l’éditeur, le curseur se place sur cette ligne.",
        "Présent en Live Preview, en Source et en Lecture. Absent si le réglage est Off, si la note n’a pas de titre, si le panneau fait moins de 80 px, sur un graphe, un PDF, ou un fichier qui n’est pas du Markdown.",
        "Réglage, page Editor : Heading navigator, valeurs Off, Floating outline, Magnifier. La palette a la commande « Editor: Cycle heading navigator (off / floating outline / magnifier) », qui les parcourt dans cet ordre.",
      ],
      en: [
        "Magnifier, the default: ticks near the pointer grow, in the manner of the Dock, and their title is written in a pill.",
        "Floating outline: a card lists the outline, indented. The current heading is bold and in the accent colour. A level-1 heading is bold as well.",
        "A click on a tick, or on a row of the card, jumps to that heading. In the editor, the caret moves to that line.",
        "Present in Live Preview, Source and Reading. Absent when the setting is Off, when the note has no heading, when the panel is under 80 px, on a graph, a PDF, or a file that is not Markdown.",
        "Setting, Editor page: Heading navigator, values Off, Floating outline, Magnifier. The palette has the command “Editor: Cycle heading navigator (off / floating outline / magnifier)”, which walks them in that order.",
      ],
    },
    demo: "nav",
    shots: [],
  },
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
          fr: "Thème clair : explorateur, note Idées, mentions liées.",
          en: "Light theme: file tree, the Idées note, linked mentions.",
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
        "La bascule est dans l’en-tête de la note. Le fondu alterne trois captures du même écran. Sans mouvement, les trois images restent l’une sous l’autre.",
      ],
      en: [
        "The switch sits in the note header. The fade alternates three captures of the same screen. Without motion, the three images stay stacked.",
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
        "Le panneau de filtres et de forces (étiquettes, pièces jointes, fichiers existants, orphelines, groupes) est dans la vue. Sur cette capture il est fermé : on voit les quatre notes et leurs liens.",
      ],
      en: [
        "The filter and forces panel (tags, attachments, existing files, orphans, groups) is part of the view. On this capture it is closed: the four notes and their links are what show.",
      ],
    },
    shots: [
      {
        src: "/media/graph.png",
        alt: {
          fr: "Graphe sombre : Brouillon, Idées, Accueil et Lecture, reliés. L’explorateur liste les mêmes notes.",
          en: "Dark graph: Brouillon, Idées, Accueil and Lecture, linked. The file tree lists the same notes.",
        },
        caption: {
          fr: "Quatre notes, et les liens entre elles.",
          en: "Four notes, and the links between them.",
        },
      },
      {
        src: "/media/backlinks.png",
        alt: {
          fr: "Note Accueil en Live Preview, panneau Linked mentions : extraits depuis Idées et Lecture, bandeau à 2 backlinks.",
          en: "Accueil in Live Preview, Linked mentions panel: excerpts from Idées and Lecture, status bar at 2 backlinks.",
        },
        caption: {
          fr: "Deux mentions liées, depuis Idées et Lecture.",
          en: "Two linked mentions, from Idées and Lecture.",
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
        "Le défilement de la première image est une animation sur la capture. Sans mouvement, l’image est entière.",
      ],
      en: [
        "Everything goes through the system `git` binary, on a side thread. The interface does not block on it. Per-vault settings live in `.obsidian/oxinote-git.json`.",
        "Four `.gitignore` toggles: ignore `.obsidian` configuration, heavy attachments (`*.pdf`, `*.mp4`, `*.zip`), the trash, and the workspace state.",
        "The pan on the first image is an animation of the capture. Without motion, the whole image is shown.",
      ],
    },
    demo: "pan",
    shots: [
      {
        src: "/media/git-history.png",
        alt: {
          fr: "Historique de la note Idées : deux commits, comparaison avec la version actuelle, bouton Restaurer cette version.",
          en: "History of the Idées note: two commits, comparison with the current version, Restore this version button.",
        },
        caption: {
          fr: "Deux commits, et le bouton pour restaurer.",
          en: "Two commits, and the button to restore.",
        },
      },
      {
        src: "/media/git-settings.png",
        alt: {
          fr: "Fenêtre Sauvegarde (Git) : sauvegarde automatique, intervalle en minutes, sauvegarde à la fermeture, message vault backup: {{date}}, cases gitignore.",
          en: "Backup (Git) window: automatic backup, interval in minutes, backup on quit, message vault backup: {{date}}, gitignore toggles.",
        },
        caption: {
          fr: "Sauvegarde automatique, intervalle, message de commit, gitignore.",
          en: "Automatic backup, interval, commit message, gitignore.",
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
          fr: "Onglet page.pdf, page 1, zoom 102 %, boutons Fit to width et Open in default app.",
          en: "Tab page.pdf, page 1, zoom 102%, Fit to width and Open in default app buttons.",
        },
        caption: {
          fr: "Page 1, zoom, largeur de page, ouverture dans l’application du système.",
          en: "Page 1, zoom, fit to width, open in the system app.",
        },
      },
      {
        src: "/media/pdf-embed.png",
        alt: {
          fr: "Note en lecture avec un PDF embarqué à la page 2, info-bulle Open test.pdf at page 2.",
          en: "Reading note with an embedded PDF on page 2, tooltip Open test.pdf at page 2.",
        },
        caption: {
          fr: "Le PDF embarqué s’ouvre à la page 2.",
          en: "The embedded PDF opens on page 2.",
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

/** Two short lines and a link label for the home blocks. */
export const homeLines: Record<string, Record<Lang, [string, string]>> = {
  nav: {
    fr: [
      "Sur le bord de chaque note, un tiret par titre.",
      "Celui que tu lis s’allume. Survole pour voir le plan, clique pour y sauter.",
    ],
    en: [
      "One tick per heading on the edge of every note.",
      "The one you’re reading lights up. Hover to see the outline, click to jump.",
    ],
  },
  vault: {
    fr: ["On désigne le dossier.", "Les notes sont déjà des fichiers Markdown."],
    en: ["Point it at the folder.", "The notes are already Markdown files."],
  },
  modes: {
    fr: ["Live Preview habille le Markdown.", "Source le montre brut, Lecture le rend."],
    en: ["Live Preview dresses the Markdown.", "Source shows it raw, Reading renders it."],
  },
  graph: {
    fr: ["Les notes se posent, les liens les retiennent.", "À côté, les mentions liées disent qui cite."],
    en: ["Notes settle, links hold them.", "Beside them, linked mentions say who cites."],
  },
  git: {
    fr: ["Le Git de la machine enregistre le vault.", "Les commits ne partent nulle part."],
    en: ["The machine’s Git records the vault.", "The commits go nowhere else."],
  },
  pdf: {
    fr: ["Un lien ouvre le PDF dans un onglet.", "Ce n’est pas un export : export écrit du HTML."],
    en: ["A link opens the PDF in a tab.", "This is not an export: export writes HTML."],
  },
  cli: {
    fr: ["Le dossier du vault est le premier argument.", "stats, search, liens, et un site HTML."],
    en: ["The vault folder is the first argument.", "stats, search, links, and an HTML site."],
  },
  install: {
    fr: ["Les fichiers sont sur la page Releases.", "Une mise à jour est comparée à SHA256SUMS.txt."],
    en: ["The files are on the Releases page.", "An update is compared to SHA256SUMS.txt."],
  },
};

export const homeMore: Record<Lang, string> = {
  fr: "Lire la page",
  en: "Read the page",
};

export const laterBand = {
  fr: {
    title: "Pas encore là.",
    text: "Bases, Canvas, les citations, le Git distant.",
    more: "La liste",
  },
  en: {
    title: "Not there yet.",
    text: "Bases, Canvas, citations, remote Git.",
    more: "The list",
  },
};

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
