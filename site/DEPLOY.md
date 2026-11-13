# Déployer la présentation OxiNote (privé, sans domaine)

Rien dans ce dossier n’a été déployé. Le propriétaire déploie lui-même, après Access.

Plateforme retenue : **Cloudflare Workers**, fichiers statiques, nom `oxinote-prive`, `workers_dev` vrai (l’adresse privée `*.workers.dev`), `preview_urls` faux, pas de domaine personnalisé. `observability.enabled` est faux : ce fichier ne demande pas de journaux Worker. Pages n’est pas le chemin retenu ; la procédure est gardée en bas, pour mémoire.

Pages du site : `/` propose les deux langues, `/fr/` et `/en/` sont les textes. Les liens `hreflang` sont en chemin depuis la racine, parce que le nom d’hôte `workers.dev` n’est pas encore choisi.

Fichiers :

| Fichier | Rôle |
|---|---|
| `wrangler.jsonc` | Nom `oxinote-prive`, `workers_dev` true, `preview_urls` false, `observability.enabled` false, `assets.directory` = `./public` |
| `public/fr/`, `public/en/` | HTML français et anglais, y compris une `404.html` par langue |
| `public/` | Choix de langue, CSS, images, `robots.txt`, `_headers`, `404.html` de repli |
| `DEPLOY.md` | Ce guide. Il n’est pas dans `public/`, donc il n’est pas publié |

Documentation utilisée :

- [Static Assets](https://developers.cloudflare.com/workers/static-assets/)
- [Configuration `assets`](https://developers.cloudflare.com/workers/wrangler/configuration/)
- [Version URLs / `preview_urls`](https://developers.cloudflare.com/workers/versions-and-deployments/version-urls/)
- [Cloudflare Access pour un Worker](https://developers.cloudflare.com/workers/configuration/cloudflare-access/)
- [Code à usage unique](https://developers.cloudflare.com/cloudflare-one/integrations/identity-providers/one-time-pin/)
- [Témoin d’autorisation Access](https://developers.cloudflare.com/cloudflare-one/access-controls/applications/http-apps/authorization-cookie/)

Le champ de configuration s’appelle encore `preview_urls`. La documentation actuelle parle de Version URLs : ce sont les adresses `<version>-<nom>.<compte>.workers.dev`.

## Avant le premier déploiement

1. L’éditeur et le responsable de la protection des renseignements personnels sont LeVibeux. Le contact public est https://github.com/LeVibeux. Le lieu indiqué est le Québec. La durée de session Access est 24 heures. Ces valeurs sont déjà dans les deux pages. Le seul espace réservé restant est `<ARNAUD_EMAIL>`, dans ce guide : l’adresse autorisée par Access. Elle ne figure pas dans `public/`.
2. Le nom du Worker est `oxinote-prive`. L’adresse sera `https://oxinote-prive.<sous-domaine-du-compte>.workers.dev`.
3. Faire l’évaluation de l’article 17 de la Loi sur le privé avant d’envoyer un renseignement personnel à Cloudflare, ou décider de ne pas déployer. Voir la page confidentialité. Ce guide ne la fait pas à votre place.
4. Vérifier en local : `python3 -m http.server --directory public 4173`, puis ouvrir `http://127.0.0.1:4173/fr/` et `http://127.0.0.1:4173/en/`.

## Déployer (quand vous l’aurez décidé)

Depuis ce dossier `site/`, avec Wrangler 4 récent (le dépôt n’en installe pas) :

```sh
npx wrangler@4 deploy
```

Wrangler demandera le compte Cloudflare. Cette commande publie vraiment : ne la lancez pas pour un essai. Il n’y a pas de domaine à ajouter. N’activez pas Zaraz ni Web Analytics sur ce Worker.

Tant qu’Access n’est pas en place, quiconque devine l’adresse `workers.dev` peut lire la page. Enchaînez avec la section suivante avant d’envoyer le lien, y compris à vous-même dans un canal qui pourrait fuir.

`preview_urls` est à `false` pour que ce déploiement ne crée pas d’URL de version. Après coup, dans le tableau de bord : **Workers & Pages** → le Worker → **Settings** → **Domains & Routes** → **Version URLs** doit rester sur Disable. Désactiver seulement `workers.dev` ne suffit pas si une URL de version a été rallumée plus tard : recoupez les deux, ou protégez les deux.

## Protéger l’adresse avec Cloudflare Access

`workers.dev` n’est pas une zone DNS de votre compte. On ne peut donc pas coller `*.workers.dev` comme nom d’hôte public d’une application auto-hébergée classique (ce sélecteur exige un domaine de la zone). Le mécanisme prévu pour ce cas est une application Access dont la destination est **le Worker**, production et previews ensemble. Cela couvre :

- `https://<nom>.<sous-domaine>.workers.dev`
- `https://<version>-<nom>.<sous-domaine>.workers.dev` si vous réactivez les Version URLs

Référence : [Protect a single application](https://developers.cloudflare.com/workers/configuration/cloudflare-access/), destination `worker` (production et previews) plutôt que `preview_worker` seul.

### 1. Zero Trust

Si Zero Trust n’est pas ouvert sur le compte : suivre [Get started](https://developers.cloudflare.com/cloudflare-one/setup/), puis revenir ici. Il faut le droit de gérer les Workers et les applications Access.

Les comptes Zero Trust créés récemment proposent Cloudflare lui-même comme fournisseur d’identité par défaut, à la place du code à usage unique. Il faut quand même ajouter le code à usage unique et le rendre obligatoire pour cette application. Ne pas laisser « toute adresse courriel » comme règle d’inclusion.

### 2. Fournisseur : code à usage unique

1. [Tableau de bord](https://dash.cloudflare.com/) → **Zero Trust** → **Integrations** → **Identity providers**.
2. **Add new identity provider** → **One-time PIN**.
3. Enregistrer.

Le courriel du code part de `noreply@notify.cloudflare.com`. S’il n’arrive pas, autoriser ce domaine chez le fournisseur de messagerie. Doc : [One-time PIN](https://developers.cloudflare.com/cloudflare-one/integrations/identity-providers/one-time-pin/).

### 3. Protéger ce Worker, production et previews

1. **Workers & Pages** → `oxinote-prive`.
2. Onglet **Access** → **Protect this Worker behind Access**.
3. Portée : **All traffic** (production et déploiements de prévisualisation). Ne pas choisir « Previews only » : l’adresse de production resterait ouverte.
4. Politique d’authentification : créer une politique **Allow** dont la seule inclusion est **Emails** = `<ARNAUD_EMAIL>`. Une seule adresse, pas un domaine entier.
5. Méthode de connexion exigée : **One-time PIN** (règle Require sur ce fournisseur, pas une inclusion ouverte à n’importe quel courriel). La documentation Access prévient qu’un Include « One-time PIN » sans liste d’adresses laisse entrer toute personne qui reçoit un code.
6. Durée de session : **24 heures**. C’est la durée déjà écrite dans les pages confidentialité.
7. **Apply Access**.

Chemin équivalent, si l’écran diffère : **Settings** → **Domains & Routes** → **Enable Cloudflare Access** sur `workers.dev` **et** sur les Version URLs, puis **Manage Cloudflare Access** pour restreindre l’adresse. Les URL de prévisualisation partagent souvent une politique de compte nommée « Cloudflare Workers Preview URLs » : cette politique doit, elle aussi, n’autoriser que `<ARNAUD_EMAIL>`. Ne pas activer « Protect all Workers » sauf si vous voulez le même verrou sur tous les Workers du compte, présents et futurs.

Contrôle après coup : en navigation privée, l’adresse doit afficher la page de connexion Access, pas le HTML. Une autre adresse courriel doit être refusée. `<ARNAUD_EMAIL>` reçoit un code et arrive sur la page.

Le témoin `CF_Authorization` est alors déposé par Access. Il est nécessaire à la session. La page ne dépose rien d’autre. Détail dans `public/fr/confidentialite.html` et `public/en/privacy.html`. `<ARNAUD_EMAIL>` ne doit pas être recopié dans ces pages.

API équivalente, si vous préférez ne pas cliquer (remplacer les identifiants, ne pas coller le jeton dans un dépôt) :

```bash
curl "https://api.cloudflare.com/client/v4/accounts/$ACCOUNT_ID/access/apps" \
  --request POST \
  --header "Authorization: Bearer $CLOUDFLARE_API_TOKEN" \
  --json '{
    "type": "self_hosted",
    "name": "OxiNote présentation privée",
    "destinations": [
      { "type": "worker", "worker_id": "<WORKER_ID>" }
    ],
    "policies": [
      {
        "decision": "allow",
        "include": [ { "email": { "email": "<ARNAUD_EMAIL>" } } ]
      }
    ]
  }'
```

Le schéma exact des politiques et de la méthode de connexion est dans l’[API Access applications](https://developers.cloudflare.com/api/resources/zero_trust/subresources/access/subresources/applications/methods/create/). Après création, ouvrir l’application dans Zero Trust et vérifier que le code à usage unique est bien la méthode exigée.

### 4. Si vous rallumez les previews

`preview_urls: false` est le réglage commité. Pour une prévisualisation malgré tout :

1. Passer `"preview_urls"` à `true` et redéployer, ou l’activer dans **Domains & Routes**.
2. Garder la destination Access de type `worker` (les deux), pas seulement la production.
3. Confirmer que la politique partagée des Preview URLs n’autorise que `<ARNAUD_EMAIL>`.
4. Cloudflare ajoute `X-Robots-Tag: noindex` sur les URL de prévisualisation `workers.dev`. La page envoie déjà la même balise, plus `robots.txt`.

Pour tout couper : remettre `preview_urls` à `false` et Disable dans le tableau de bord.

## Pages — non retenu

La plateforme retenue est Workers. La procédure Pages reste ici si le choix était rouvert :

1. Créer un projet Pages dont le dossier publié est `site/public`. Pas de commande de build.
2. Ne pas ajouter de domaine personnalisé. L’adresse sera `https://<projet>.pages.dev`.
3. **Workers & Pages** → le projet → **Settings** → **General** → désactiver l’accès public des déploiements de prévisualisation, ou les couvrir par Access. Les aperçus Pages (`<hash>.<projet>.pages.dev`) sont lisibles par quiconque a le lien tant qu’Access ne les couvre pas.
4. Activer Access sur la production **et** sur les previews, même politique : Allow, email `<ARNAUD_EMAIL>` seulement, One-time PIN en méthode exigée.
5. Le sélecteur « nom d’hôte public » d’une application auto-hébergée demande une zone à vous. `pages.dev` n’en est pas une : utiliser le bouton Access du projet Pages, qui crée l’application pour `*.pages.dev` de ce projet, plutôt que de déclarer `*.pages.dev` à la main.

## Ce qu’il ne faut pas faire

- Pas de `wrangler deploy` depuis un agent ou une CI tant que vous ne l’avez pas demandé.
- Pas de domaine personnalisé, pas d’annonce, pas de Web Analytics, pas de Zaraz.
- Ne pas committer de jeton API, de courriel réel à la place de `<ARNAUD_EMAIL>`, ni de secret dans `wrangler.jsonc`.
