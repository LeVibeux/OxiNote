<p align="center">
  <img src="assets/logo/obsirust-icon-256.png" alt="Logo ObsiRust" width="160">
</p>

<h1 align="center">ObsiRust</h1>

Réimplémentation d'Obsidian en Rust, compatible avec les vaults existants.

Voir [PLAN.md](PLAN.md) pour l'architecture et la feuille de route.

## Installation

Téléchargez l'archive de votre système dans les
[Releases](https://github.com/LeVibeux/ObsiRust/releases) (macOS Apple Silicon et
Intel, Windows, Linux). Elle contient l'application et le CLI `obsirust`.

Pour publier une version : poussez un tag `vX.Y.Z` ; le workflow
`.github/workflows/release.yml` compile et publie la release.

## Développement

```sh
cargo test --workspace
cargo run -p ors-cli -- stats examples/sample-vault
cargo run -p ors-app -- examples/sample-vault
```
