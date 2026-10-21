# Logo ObsiRust

| Fichier | Usage |
|---|---|
| `obsirust-logo.jpg` | Illustration originale (1408×1408), source de tous les autres fichiers |
| `obsirust-icon.png` | Icône d'application 1024×1024, coins arrondis, fond transparent |
| `obsirust-icon-{16,32,48,64,128,256,512}.png` | Icône déclinée par taille (16–48 recadrées plus serré pour rester lisibles) |
| `obsirust.ico` | Icône Windows multi-tailles (16 à 256) |
| `obsirust-social.jpg` | Aperçu social GitHub 1280×640 (*Settings → Social preview*) |

L'icône de fenêtre de `obsirust-app` est une copie de `obsirust-icon-256.png`
dans `crates/ors-app/assets/obsirust-icon.png` (intégrée au binaire).

Installation sous Linux (menu des applications) :

```sh
install -Dm644 assets/logo/obsirust-icon-256.png ~/.local/share/icons/hicolor/256x256/apps/obsirust.png
install -Dm644 assets/linux/obsirust.desktop ~/.local/share/applications/obsirust.desktop
```
