<p align="center">
  <img src="assets/logo/obsirust-icon-256.png" alt="Logo ObsiRust" width="160">
</p>

<h1 align="center">ObsiRust</h1>

Réimplémentation d'Obsidian en Rust, compatible avec les vaults existants.

Voir [PLAN.md](PLAN.md) pour l'architecture et la feuille de route.

```sh
cargo test --workspace
cargo run -p ors-cli -- stats examples/sample-vault
cargo run -p ors-app -- examples/sample-vault
```
