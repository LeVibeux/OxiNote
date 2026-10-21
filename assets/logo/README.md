# Logo Oxinote

| Fichier | Usage |
|---|---|
| `oxinote-logo.jpg` | Illustration originale (1408×1408), source de tous les autres fichiers |
| `oxinote-icon.png` | Icône d'application 1024×1024, coins arrondis, fond transparent |
| `oxinote-icon-{16,32,48,64,128,256,512}.png` | Icône déclinée par taille (16–48 recadrées plus serré pour rester lisibles) |
| `oxinote.ico` | Icône Windows multi-tailles (16 à 256) |
| `oxinote-social.jpg` | Aperçu social GitHub 1280×640 (*Settings → Social preview*) |

L'icône de fenêtre de `oxinote-app` est une copie de `oxinote-icon-256.png`
dans `crates/oxi-app/assets/oxinote-icon.png` (intégrée au binaire).

Installation sous Linux (menu des applications) :

```sh
install -Dm644 assets/logo/oxinote-icon-256.png ~/.local/share/icons/hicolor/256x256/apps/oxinote.png
install -Dm644 assets/linux/oxinote.desktop ~/.local/share/applications/oxinote.desktop
```
