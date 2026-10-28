# Keymap Studio

[English](README.md) | 日本語

Windows 11 の [PowerToys Keyboard Manager](https://learn.microsoft.com/ja-jp/windows/powertoys/keyboard-manager) のキーリマップを、キーボードの図の上で視覚的に編集できるアプリです。リマップのセットを丸ごと、ホットキーで切り替えることもできます。

![Keymap Studio](docs/images/main.png)

> アプリの画面は現在英語のみです(日本語化を予定しています)。以下ではボタン名を英語のまま書いています。

## 特長

- **キーボードの図の上で編集**: US 配列(ANSI 104 キー)の図でキーをクリックしてリマップします。リマップしたキーには変換先(例: `F12 → A`)が表示されます。未保存の変更は色を変えて示します。
- **変換先を選びやすい**: 名前で検索する、カテゴリ別の一覧から選ぶ、図の上で変換先のキーをクリックする、の 3 通りで選べます。
- **テンキーを正しく扱う**: テンキーの 1 つのキーが NumLock の状態によって別のキーコードを送ることを区別します。たとえば、Shift を押していても常にテンキーで数字を入力させる設定ができます(MMO などのゲーム向け)。
- **保存すると即座に反映**: 保存のたびに PowerToys へ読み直しを通知します。Keyboard Manager をオフ→オンする必要はありません。
- **安全な保存**:
  - 保存のたびに、上書きする前のファイルを残します(直近 10 回分)。
  - 一時ファイルに書いてから置き換えます。
  - PowerToys のエディターと同じロックファイルを使います。
  - 読み込んだあとに他のプログラムがファイルを変更していたら、上書きせずに知らせます。
- **いつでも元に戻せる**: 初回起動時に、それまでの PowerToys の設定をバックアップします。**Restore settings from before installation…** でいつでも戻せます。
- **キーマップ**:
  - リマップのセットを名前を付けて保存し、ワンクリックで適用できます。
  - 1 キーマップ = 1 つの JSON ファイルなので、書き出して別の PC に取り込めます。
- **プリセット**: [大西配列](https://o24.works/layout/)を同梱しています。そのまま適用することも、今の編集に重ねることもできます。
- **トレイ・ホットキー・自動起動**:
  - タスクトレイに常駐します。
  - キーマップやプリセットごとにグローバルホットキー(例: Ctrl+Alt+1)を割り当てられます。
  - 切り替えたら通知で知らせます。
  - Windows へのサインイン時にトレイで起動できます。
- **ポータブル**: exe 1 つだけで、インストーラーはありません。データはすべて exe と同じフォルダに置きます。

## 動作環境

- Windows 11(x86_64)
- [PowerToys](https://learn.microsoft.com/ja-jp/windows/powertoys/install)。Keyboard Manager を一度は有効にしておいてください。
- Keyboard Manager の新しいエディター(`useNewEditor: true`)の環境で動作を確認しています。

PowerToys や Keyboard Manager の設定が見つからないときは、何も編集せずに対処方法を表示します。

## はじめかた

1. [Releases](https://github.com/vitivalkoinen/keymap-studio/releases) から `keymap-studio.exe` をダウンロードします。
2. 書き込みできるフォルダ(例: `C:\Users\<ユーザー名>\keymap-studio\`)に置きます。データは同じフォルダに保存されます。
3. 実行します。
   - exe にはコード署名がないため、Windows SmartScreen の警告が出ることがあります。「詳細情報」→「実行」を選んでください。

初回起動時、何かを変更する前に、今の PowerToys Keyboard Manager の設定をバックアップします。

## 使い方

### リマップの編集

1. 図の上で、押すキー(リマップ元)をクリックします。
2. 変換先を選びます。方法は次のどれでもかまいません。
   - 一覧を名前で検索する
   - カテゴリから選ぶ
   - **Pick on keyboard** を押してから、図の上で変換先のキーをクリックする
3. **Save** を押します。未保存の変更がある間は、ボタンが強調表示されます。保存するとすぐに PowerToys に反映されます。

キーの色は、リマップなし・保存済みのリマップ・未保存の変更、の 3 つの状態を表します。

### キーマップとプリセット

- **Save current as new keymap…**: 画面上のリマップを、名前を付けたキーマップとして保存します。
- **Apply**: PowerToys のリマップをすべてそのキーマップの内容に置き換え、すぐに反映します。
- **Edit**: キーマップを編集画面に読み込みます。反映はされないので、**Save** を押すと反映されます。
- **Overwrite** / **Rename** / **Delete** / **Export…** / **Import…**: キーマップのファイルを管理します。
- プリセットはそのまま適用できるほか、**Add to current edits** で今の編集に重ねられます。例: テンキーの数字固定 + 大西配列。重ねた結果は自分のキーマップとして保存できます。
- 今の PowerToys の設定と一致するキーマップ・プリセットには **In use** と表示されます。

注意: PowerToys のキーリマップはショートカットにも効きます。たとえば大西配列では、物理キーの S を Ctrl と一緒に押すと Ctrl+I になります。

### トレイとホットキー

- ウィンドウの × を押すと、ウィンドウを隠してトレイに残ります。編集内容は保持されます。
- 終了するときは、トレイのメニューの **Exit** を選びます。
- トレイのメニューにはキーマップとプリセットが並んでいて、選ぶとそれに切り替わります。
- トレイアイコンをダブルクリックすると、ウィンドウが開きます。
- キーマップやプリセットの **Set hotkey…** で、グローバルホットキーを割り当てます。
  - Ctrl・Alt・Shift・Win のどれかを含む組み合わせにしてください。
  - ホットキーで切り替えると、ウィンドウの未保存の編集は破棄されます。破棄したときは、通知でそのことを知らせます。
- **Tray, hotkeys and startup** で、次の 2 つをオフにできます。
  - サインイン時の自動起動(既定はオン)
  - 切り替えの通知
- 同時に起動できるのは 1 つだけです。もう一度起動すると、起動中のウィンドウが前面に出ます。

### 元の設定に戻す

**Restore settings from before installation…** を押すと、初回起動時にバックアップしたリマップの定義を書き戻します。実行前に確認があります。Keymap Studio で行った変更をすべて取り消したいときに使ってください。

## ファイル

データはすべて `keymap-studio.exe` と同じフォルダに保存されます。

| パス | 内容 |
|---|---|
| `keymaps\` | 保存したキーマップ(1 つにつき 1 つの JSON ファイル) |
| `initial-backup\` | 初回起動時にバックアップした PowerToys の設定(上書きされません) |
| `backups\` | 保存のたびに残す、上書きする前のリマップのファイル(直近 10 回分) |
| `keymap-studio.settings.json` | ホットキー・自動起動・通知の設定 |
| `keymap-studio.log` | 動作ログ |

PowerToys のファイルのうち、Keymap Studio が書き込むのはリマップの定義 `%LOCALAPPDATA%\Microsoft\PowerToys\Keyboard Manager\default.json` だけです。PowerToys の `settings.json` と `editorSettings.json` は変更しません。

別の PC へ移すときは、`keymaps\` フォルダをコピーするか、**Export…** / **Import…** を使ってください。

## アンインストール

1. 元のリマップに戻したい場合は、**Restore settings from before installation…** を押します。
2. **Start automatically in the tray when signing in to Windows** をオフにします。`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` の `Keymap Studio` が削除されます。
3. トレイのメニューの **Exit** で終了します。
4. `keymap-studio.exe` があるフォルダを削除します。

## ソースからのビルド

Rust のツールチェーン(edition 2024)が必要です。

```sh
# Windows の場合
cargo build --release

# Linux / WSL からクロスコンパイルする場合(mingw-w64 が必要)
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

exe は `target/release/` または `target/x86_64-pc-windows-gnu/release/` にできます。チェックは次のコマンドで実行します。

```sh
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

詳しい仕様は [SPEC.md](SPEC.md) にあります。

## 今後の予定

- ショートカットのリマップ、テキストへのリマップ
- 日本語の UI
- JIS 配列

## ライセンス

[MIT](LICENSE)

このプロジェクトは Microsoft とは関係がなく、Microsoft の承認を受けたものでもありません。PowerToys は Microsoft Corporation の商標です。大西配列とその作者については <https://o24.works/layout/> を参照してください。
