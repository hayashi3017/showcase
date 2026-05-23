# Algorithm Showcase

`egui` / `eframe` を使って、さまざまなアルゴリズムをインタラクティブに表示する Rust ワークスペースです。各アルゴリズムは原則として独立した Cargo クレートとして追加し、デスクトップ実行と GitHub Pages 向け WebAssembly ビルドの両方を保ちます。

## Current Visualizers

- `wfc-road-visualizer`: Wave Function Collapse による道路生成の可視化

## Repository Layout

```text
.
├── Cargo.toml                  # workspace manifest
├── README.md
├── docs/
│   └── adding-visualizer.md    # 新しい可視化を追加する手順
└── wfc-road-visualizer/
    ├── Cargo.toml
    ├── index.html              # Trunk / GitHub Pages entry point
    └── src/main.rs
```

## Development Commands

```sh
cargo run -p wfc-road-visualizer
cargo check --workspace
cargo test --workspace
cargo fmt
cargo clippy --all-targets --all-features
```

Web ビルドを確認する場合は、対象クレートのディレクトリで Trunk を実行します。

```sh
cd wfc-road-visualizer
trunk build --release --public-url /showcase/
```

ローカルの `trunk 0.21.x` で `NO_COLOR=1` による `--no-color` エラーが出る場合は、検証時だけ `env -u NO_COLOR trunk build ...` としてください。

## Adding Algorithms

新しいアルゴリズムは、ワークスペース直下に `<algorithm>-visualizer` 形式のクレートを追加します。実装前に [docs/adding-visualizer.md](docs/adding-visualizer.md) のチェックリストを確認してください。

最低限そろえる項目:

- `Cargo.toml` の workspace `members` への追加
- `eframe` / `egui` ベースの native entry point
- GitHub Pages 用の `index.html` と wasm entry point
- アルゴリズムの状態更新、描画、操作 UI の分離
- deterministic なコアロジックのテスト

## Deployment

GitHub Actions で `main` への push 時に GitHub Pages へデプロイします。Pages が未有効の場合、`.github/workflows/pages.yml` は `actions/configure-pages` の `enablement` で有効化を試みます。初回だけ `repo` scope または Pages write permission を持つ token を `PAGES_TOKEN` secret に設定してください。すでに Pages が有効なら通常の `GITHUB_TOKEN` で動作します。新しい visualizer を公開対象にする場合は、ワークフローと公開ページのリンク一覧も更新します。
