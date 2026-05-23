# Algorithm Showcase

`egui` / `eframe` を使って、さまざまなアルゴリズムをインタラクティブに表示する Rust ワークスペースです。各アルゴリズムは原則として独立した Cargo クレートとして追加し、デスクトップ実行と GitHub Pages 向け WebAssembly ビルドの両方を保ちます。

## Current Visualizers

- `wfc-road-visualizer`: Wave Function Collapse による道路生成の可視化
- `search-index-visualizer`: 転置インデックス、TF-IDF、コサイン類似度による検索ランキングの可視化

## Algorithm Notes

### WFC Road Visualizer

Wave Function Collapse は、各セルが取り得るタイル候補を集合として持ち、制約を満たすように候補を少しずつ削っていく生成アルゴリズムです。この visualizer では 4 方向の道路接続を 4bit の mask として表し、隣り合うタイルの辺が一致する組み合わせだけを許可します。

処理は大きく 3 段階です。まず外周から道路が外へ出ないよう境界制約を適用します。次にいくつかの内部セルへ道路タイルを seed として置き、生成結果が空白に偏りすぎないようにします。以降は候補数が最も少ない未確定セルを選び、重みに従って 1 タイルへ collapse し、その影響を隣接セルへ伝播します。画面では候補数、直近で collapse したセル、伝播で更新されたセルを確認できます。

### Search Index Visualizer

Search Index Visualizer は、小さな全文検索エンジンのランキング処理を分解して表示します。文書の title と body を解析し、ASCII の単語と日本語/CJK の n-gram token に変換します。各 token について文書ごとの出現回数を数え、document frequency から IDF を計算します。

文書ベクトルは TF-IDF 重みを L2 正規化したうえで転置インデックスに保存します。検索時はクエリも同じ analyzer と IDF でベクトル化し、クエリに含まれる term の posting list だけを走査します。文書ベクトルとクエリベクトルはどちらも正規化済みなので、term ごとの積を足した値が cosine similarity になります。画面では query token、検索順位、term ごとのスコア寄与、文書 token、辞書の df/idf を確認できます。

## Repository Layout

```text
.
├── Cargo.toml                  # workspace manifest
├── README.md
├── docs/
│   └── adding-visualizer.md    # 新しい可視化を追加する手順
├── site/
│   └── index.html              # GitHub Pages のダッシュボード
├── search-index-visualizer/
│   ├── Cargo.toml
│   ├── index.html
│   └── src/main.rs
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
trunk build --release --public-url /showcase/wfc-road-visualizer/
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

GitHub Actions で `main` への push 時に GitHub Pages へデプロイします。`/` は `site/index.html` のダッシュボード、各 visualizer は `/wfc-road-visualizer/` などの個別パスで公開します。`main` 向け PR では check と Trunk build だけを実行し、deploy は行いません。Pages の source はリポジトリ設定で **GitHub Actions** を選択してください。新しい visualizer を公開対象にする場合は、ワークフローと公開ページのリンク一覧も更新します。
