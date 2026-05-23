# Adding an Algorithm Visualizer

このドキュメントは、ワークスペースに新しい `egui` ベースのアルゴリズム可視化を追加するための手順です。

## Naming and Scope

クレート名は `<algorithm>-visualizer` を基本形にします。例:

- `astar-visualizer`
- `dijkstra-visualizer`
- `sort-visualizer`

1 クレートは 1 つの主要アルゴリズムまたは密接に関連するアルゴリズム群に絞ります。比較表示が主目的の場合だけ、複数アルゴリズムを同じクレートにまとめます。

## Create the Crate

```sh
cargo new <algorithm>-visualizer --bin
```

ルートの `Cargo.toml` に追加します。

```toml
[workspace]
resolver = "2"
members = [
    "wfc-road-visualizer",
    "<algorithm>-visualizer",
]
```

対象クレートには少なくとも次を追加します。

```toml
[dependencies]
eframe = "0.31"
egui = "0.31"

[target.'cfg(target_arch = "wasm32")'.dependencies]
log = "0.4"
wasm-bindgen-futures = "0.4"
```

## Implementation Checklist

- `App` 構造体に UI 状態、アルゴリズム状態、実行モードを持たせる。
- アルゴリズム本体は描画コードから分離し、テストしやすい関数や型にする。
- `Step`、`Auto`、`Reset`、速度調整、seed やサイズなどの主要パラメータを用意する。
- 可能なら同じ入力から同じ結果になる deterministic な実装にする。
- 大きい入力でも UI が固まりにくいよう、1 フレーム内の処理量に上限を置く。
- エラーや完了状態を画面上に表示する。

## Native and Web Entry Points

native と wasm は `cfg` で分けます。

```rust
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    eframe::run_native(
        "Visualizer",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::{wasm_bindgen::JsCast as _, web_sys};

    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("app"))
            .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("missing #app canvas");

        eframe::WebRunner::new()
            .start(canvas, eframe::WebOptions::default(), Box::new(|_cc| {
                Ok(Box::new(App::default()))
            }))
            .await
            .expect("failed to start eframe web app");
    });
}
```

`index.html` もクレート直下に置きます。

```html
<canvas id="app"></canvas>
<link data-trunk rel="rust" data-bin="<algorithm>-visualizer" />
```

## Testing and Validation

追加前後に次を実行します。

```sh
cargo fmt
cargo check --workspace
cargo check --target wasm32-unknown-unknown -p <algorithm>-visualizer
cargo test --workspace
```

Web 配布物は対象クレートで確認します。

```sh
cd <algorithm>-visualizer
trunk build --release --public-url /showcase/
```

## Documentation Updates

新しい可視化を追加したら、次も更新します。

- `README.md` の Current Visualizers
- GitHub Pages で公開する対象一覧やリンク
- `.github/workflows/pages.yml` の build 対象
- 操作方法が特殊な場合はクレート内 README または `docs/` の補足

## Pages Deployment Checklist

GitHub Pages で複数 visualizer を公開する場合は、各クレートを Trunk でビルドし、成果物を 1 つの Pages artifact にまとめます。追加時は次を確認してください。

- 生成先の `dist/` は `.gitignore` に追加し、コミットしない。
- `--public-url` は GitHub Pages の base path に合わせる。
- 公開トップページから新しい visualizer へ移動できるリンクを用意する。
- CI で `cargo check --workspace` と対象クレートの Trunk build が通る。

## Review Points

Pull request では、実装内容、操作方法、検証コマンド、UI 変更のスクリーンショットを含めます。環境回避策を追加した場合は、観測したエラーと回避理由をコメントに残してください。
