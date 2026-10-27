# keymap-studio 仕様

Windows 11 の PowerToys Keyboard Manager のキーリマップを、キーボード図の上で視覚的に編集するデスクトップアプリ。

> 「未決定」とある項目は実装前にユーザーと合意すること。推測で埋めない。

## 1. 対象・前提

- 対象 OS: Windows 11(x86_64)
- 対象: PowerToys Keyboard Manager(新エディタ `useNewEditor: true` の環境で確認済み)
- 配布形態: 単一の exe(インストーラは未決定)

## 2. 技術スタック

| 用途 | 採用 |
|---|---|
| 言語 | Rust(edition 2024) |
| GUI | egui / eframe |
| JSON | serde / serde_json |
| パス解決 | dirs(`data_local_dir()`) |
| Windows 向けビルド | cargo-xwin(ローカル)、GitHub Actions windows-latest(CI) — 未整備 |

## 3. 設定ファイル

所在: `%LOCALAPPDATA%\Microsoft\PowerToys\Keyboard Manager\`

### 3.1 `default.json`(エンジンが読むリマップ定義)

```json
{
  "remapKeys":            { "inProcess": [ { "originalKeys": "2147483685", "newRemapKeys": "100" } ] },
  "remapKeysToText":      { "inProcess": [] },
  "remapShortcuts":       { "global": [], "appSpecific": [] },
  "remapShortcutsToText": { "global": [], "appSpecific": [] }
}
```

- キーは 10 進の仮想キーコード(VK)を文字列にしたもの。複数キー(ショートカット)は `;` 区切り
- 最上位ビット `0x80000000` は**拡張キー**を表すフラグ。例: `2147483685` = `0x80000025` = 矢印 ←(テンキー 4 と区別される)
- 読み書きでは未知のフィールドをすべて保持し、書き戻しで失わないこと

### 3.2 `editorSettings.json`(新エディタの状態)

- `ShortcutSettingsDictionary` に GUID をキーとしてエントリを持つ
- キーコードは**符号付き** 32bit 整数の文字列(例: `-2147483603` = `0x8000002D`)
- `OperationType` / `TargetApp` / `TargetText` / `ProgramPath` / `UriToOpen` / `Profiles` / `IsActive` などのフィールドがある

### 3.3 `settings.json`

- `activeConfiguration`(既定 `"default"`)、`useNewEditor` など

## 4. 機能

### 4.1 実装済み

- 設定ファイルの読み込み(既定のパス、またはコマンドライン引数 1 つ目で指定したパス)
- キー→キーのリマップの一覧表示(VK を 16 進表示)

### 4.2 予定

1. キーボード図の描画と、キーをクリックして選択する操作
2. VK と表示名の対応表(拡張キーを区別する)
3. キー→キーのリマップの追加・変更・削除
4. ショートカットのリマップ(グローバル/アプリごと)
5. テキストへのリマップ
6. 保存

## 5. 未決定事項

- **保存方式**: `default.json` だけを書き換えると、新エディタの `editorSettings.json` と内容が食い違う可能性がある。両方を書き換えるか、どちらかを正とするか
- **反映**: ファイルを書き換えたとき、PowerToys のエンジンがそれを検知して反映するか(要検証)
- **書き込みの安全性**: バックアップの作成、一時ファイルへ書いてから置き換える方式の採用、`editorTransaction.lock` の扱い
- **キーボードの配列**: JIS / US のどちらに対応するか、切り替えられるようにするか
- **UI の言語**: 日本語にする場合は CJK フォントの埋め込みが必要
- **対象とする操作の種類**: プログラムの起動、URI を開く、など新エディタの機能にどこまで対応するか
