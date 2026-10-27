# CLAUDE.md

PowerToys Keyboard Manager のキーリマップを視覚的に編集する Windows 向けデスクトップアプリ(Rust + egui)。

## ドキュメントの役割

- **SPEC.md**: 仕様の唯一の置き場。仕様の追加・変更・判明した事実はここへ書く
- **CLAUDE.md**(このファイル): Claude への作業上の指示だけを書く。仕様は書かない
- SPEC.md の「未決定事項」は推測で実装しない。ユーザーに確認し、決まったら SPEC.md を更新してから実装する

## 開発の進め方(エージェントの分担)

メインセッションが取りまとめ役として次の流れを回す。

1. タスクを SPEC.md と照らし合わせ、範囲と完了条件を決める
2. `implementer` エージェントに実装を依頼する(SPEC.md の該当節と完了条件を渡す)
3. `reviewer` エージェントに差分のレビューを依頼する
4. 指摘があれば `implementer` に修正を依頼し、レビューを通るまで 2〜3 を繰り返す
5. 結果をユーザーに報告する。コミットはユーザーの指示があるときだけ行う

メインセッションは原則として自分でコードを書かない。仕様の更新、エージェントへの依頼、結果の確認・報告を担う。

## 開発環境

- WSL2 上で開発し、成果物の対象は Windows
- PowerToys の実際の設定ファイル: `/mnt/c/Users/snow7/AppData/Local/Microsoft/PowerToys/Keyboard Manager/`
  - **読むのは自由。書き込むときは事前にユーザーの許可を得る**(ユーザーが実際に使っている設定のため)。テストではリポジトリ内のフィクスチャか一時ディレクトリを使う
- シェルの `cd` は zoxide に置き換えられているため、`builtin cd` を使うか絶対パスで操作する

## コマンド

```sh
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
# WSLg での起動(GPU を使った EGL の初期化に失敗するためソフトウェア描画を指定する)
LIBGL_ALWAYS_SOFTWARE=1 cargo run -- "/mnt/c/Users/snow7/AppData/Local/Microsoft/PowerToys/Keyboard Manager/default.json"
```

変更を完了とする条件: `cargo test`・`cargo clippy -- -D warnings`・`cargo fmt --check` がすべて通ること。

## コーディング規約

- コメントとドキュメントは日本語。識別子は英語
- 設定ファイルの読み書きでは未知のフィールドを必ず保持する(`#[serde(flatten)] extra`)
- 設定ファイルの形式に関わる変更にはユニットテストを付ける
- egui / eframe は 0.37 系。API の変更が多いので、推測で書かずに `~/.cargo/registry/src/` のソースで確認する
