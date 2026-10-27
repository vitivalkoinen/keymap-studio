# keymap-studio 仕様

Windows 11 の PowerToys Keyboard Manager のキーリマップを、キーボード図の上で視覚的に編集するデスクトップアプリ。

> 「未決定」とある項目は実装前にユーザーと合意すること。推測で埋めない。

## 1. 対象・前提

- 対象 OS: Windows 11(x86_64)
- 対象: PowerToys Keyboard Manager(新エディタ `useNewEditor: true` の環境で確認済み)
- 配布形態: ポータブル版(単一の exe)のみ。インストーラは用意しない
- 本アプリのデータ(保存したキーマップ、バックアップ、本アプリの設定)は exe と同じフォルダに置く。書き込めない場所に置かれたときはエラーで知らせる

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
- 最上位ビット `0x80000000` は**テンキー由来**を表すフラグ(PowerToys のソースでの名前は `numpadOriginBit`。`src/common/interop/keyboard_layout.cpp`)。Windows の「拡張キー」とは意味が逆なので注意
  - 例: `2147483685` = `0x80000025` = テンキーの 4(NumLock オフ時の ←)。フラグのない `37`(`0x25`)はナビゲーションクラスタの矢印 ←
  - フラグが付きうる VK: Left / Right / Up / Down / Insert / Delete / PgUp / PgDn / Home / End / Enter / Divide(テンキーの /)
  - エンジンは押されたキーを次の規則で符号化してから照合する(`src/modules/keyboardmanager/common/Helpers.cpp` の `EncodeKeyNumpadOrigin`)
    - Left / Right / Up / Down / Insert / Delete / PgUp / PgDn / Home / End: Windows の拡張キーで**ない**とき(= テンキー)にフラグを付ける
    - Enter / Divide: Windows の拡張キーで**ある**とき(= テンキー)にフラグを付ける
    - したがってテンキーの / は常に `0x8000006F` として届く。フラグなしの `0x6F` をリマップ元にしても効かない
- テンキーの 1 つの物理キーは、NumLock の状態によって複数の VK に対応する。例: テンキーの 4 は `VK_NUMPAD4`(`0x64`)と `0x80000025`、テンキーの 5 は `VK_NUMPAD5`(`0x65`)と `VK_CLEAR`(`0x0C`、フラグなし)、テンキーの . は `VK_DECIMAL`(`0x6E`)と `0x8000002E`
- PowerToys 独自の値(`src/common/interop/shared_constants.h`): `0x100` = `VK_DISABLED`(表示名 "Disable"。キーの無効化)、`0x104` = `VK_WIN_BOTH`(表示名 "Win"。左右どちらの Win でも)
- 想定する使い方の例: NumLock や Shift の状態にかかわらずテンキーで数字を入力させる(Shift+テンキーがナビゲーション操作にならず Shift+Num1 などとして届く)ために、テンキー由来の各 VK をテンキーの数字へリマップする(MMO などのゲームでの利用)
- 読み書きでは未知のフィールドをすべて保持し、書き戻しで失わないこと

### 3.2 `editorSettings.json`(新エディタの状態)

- `ShortcutSettingsDictionary` に GUID をキーとしてエントリを持つ
- キーコードは**符号付き** 32bit 整数の文字列(例: `-2147483603` = `0x8000002D`)
- `OperationType` / `TargetApp` / `TargetText` / `ProgramPath` / `UriToOpen` / `Profiles` / `IsActive` などのフィールドがある

### 3.3 `settings.json`

- 値は `properties` の下に `{ "value": ... }` の形で入っている。例: `properties.activeConfiguration.value`(既定 `"default"`)、`properties.keyboardConfigurations.value`(配列)、`properties.useNewEditor`(真偽値がそのまま入る)
- `activeConfiguration` の値は、ファイル名として安全なもの(空でない、パス区切り `\` `/`・`..`・`:`・`< > " | ? *`・制御文字を含まない、Windows の予約デバイス名 CON / PRN / AUX / NUL / COM0–9 / LPT0–9 でない)だけを使う。`settings.json` が存在しないか値がないときは `default` とみなす。存在するのに読めない・解析できないときはエラーにする。安全でなければ保存を中止してユーザーに知らせる

## 4. 機能

### 4.1 実装済み

- 設定ファイルの読み込み(既定のパス、またはコマンドライン引数 1 つ目で指定したパス)
- キー→キーのリマップの一覧表示(表示名で表示)
- VK と英語の表示名の対応表(テンキー由来のフラグを区別する。PowerToys 独自の値 0x100 / 0x104 を含む)
- US 配列(ANSI 104 キー)のキーボード図の描画と、キーをクリックして選択する操作
  - テンキーの各キーは NumLock オン/オフ時の VK をすべて持ち、どれかがリマップ元ならそのキーを色で示す
  - 選択中のキーの名前・コード・関係するリマップを表示する
- UI 文字列を言語ごとの対応表から引く土台(英語のみ)
- キー→キーのリマップの追加・変更・削除(リマップ先は表示名つきの一覧から選ぶ。テンキー由来のフラグ付きコードはリマップ先に選べない)
- 保存(§4.8)。未保存のまま閉じるときの確認、外部の変更を検知したあとの読み直し

### 4.2 予定

1. ショートカットのリマップ(グローバル/アプリごと)
2. テキストへのリマップ
3. キーマップの切り替え(§4.5)
4. キーマップのプリセット(§4.6)
5. 導入前の設定のバックアップと復元(§4.7)
6. 左右を区別しない Ctrl / Alt / Shift(0x10–0x12)と Win(0x104)をキーボード図でどう示すか(未決定)

### 4.3 キーボードの配列

- 初期実装は US 配列のみ
- 将来 JIS 配列を選べるようにする(実装は後回し)。配列の定義は差し替えられる形にしておく

### 4.4 UI の言語

- 既定は英語
- 日本語へのローカライズに対応できる作りにする(UI 文字列をコードに直書きせず、言語ごとの対応表から引く)
- 対応する言語は英語と日本語のみ。他の言語は対象外
- 日本語表示には CJK フォントの埋め込みが必要

### 4.5 キーマップの切り替え

- あらかじめ保存しておいた複数のキーマップのうち、どれかにすぐ切り替えられるようにする
- 切り替えはアプリの画面からと、ホットキーからの両方で行える
- キーマップは本アプリ独自に、データフォルダ(§1)へ保存する。切り替えは、選んだキーマップの内容で `default.json` を書き換えて保存する(§4.8 の保存処理をそのまま使う)
- ホットキーはキーマップごとに割り当てる(例: Ctrl+Alt+1 で 1 つ目のキーマップ)
- ホットキーを受け付けるため、タスクトレイに常駐する。ウィンドウを閉じてもトレイに残り、トレイのメニューからも切り替えられる
- Windows へのサインイン時に自動で起動し、ウィンドウは開かずにタスクトレイにだけ表示する
  - 登録先は `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`(管理者権限が不要で、ポータブル版でも使える)。値には exe のフルパスと、トレイだけで起動することを示す引数を入れる
  - 既定で有効。アプリの設定で無効にできる(無効にしたら登録を消す)
  - ポータブル版で exe が移動されても追従するよう、有効なら起動のたびに登録内容を現在の exe のパスに合わせる
- ホットキーの登録に失敗したとき(他のアプリと衝突しているなど)は、どのキーマップのホットキーが使えないかを画面で知らせる

### 4.6 キーマップのプリセット

- よく知られたキー配列をプリセットとして同梱し、選ぶだけで適用できるようにする
- 初期の同梱プリセットは大西配列のみ(US 配列用)
  - 大西配列の利用条件を明示した資料は見つからなかったが、公開ページの表現からパブリックドメインに近い扱いと判断し、非商用であることも踏まえて同梱する(2026-10-28、ユーザーの判断と責任による)
- 大西配列の定義(出典: https://o24.works/layout/ 、2026-10-28 確認)。US 配列の物理キーの位置に対して、次の文字を割り当てる(左から右)

  | 行 | QWERTY での物理キー | 大西配列 |
  |---|---|---|
  | 上段 | Q W E R T Y U I O P | Q L U , . F W R Y P |
  | 中段 | A S D F G H J K L ; | E I A O - K T N S H |
  | 下段 | Z X C V B N M , . / | Z X C V ; G D M J B |

  - キー→キーのリマップ(`remapKeys.inProcess`)として表す。物理キーと同じ文字になるキー(Q、P、Z、X、C、V など)にはリマップを作らない
  - この 30 キー以外(数字の段、元の `-` キーなど)は変えない
  - PowerToys のキー→キーのリマップは、Ctrl などと組み合わせたショートカットにも効く(例: 物理キー C は C のままだが、物理キー S を押した Ctrl+S は Ctrl+I になる)。これはプリセットの性質として画面で注意書きを出す

### 4.7 導入前の設定のバックアップと復元

- インストーラがないため、「導入時」は本アプリの初回起動時とする。初回起動時に、その時点の PowerToys Keyboard Manager の設定をバックアップする
- バックアップした設定にいつでも戻せるようにする。そのための専用のボタン(「導入前の設定に戻す」)を用意し、押したら確認してから戻す
- アンインストーラもないため、「アンインストール時に戻すか尋ねる」は、この専用のボタンで代える
- 初回起動時のバックアップの対象は `settings.json`・`editorSettings.json`・リマップの定義ファイル(`activeConfiguration` のもの)の 3 つ。データフォルダ(§1)に保存し、以後は上書きしない
- 戻すときはリマップの定義ファイルだけを §4.8 の保存処理で書き戻す(`editorSettings.json` は PowerToys の新エディタが起動時に合わせ直すため)。ほかの 2 つは控えとして残す

### 4.8 保存

- 書き込むのは `default.json` だけ。エンジンが読むのはこのファイルなので、これを正とする
  - 根拠: PowerToys の新エディタは起動するたびに、エンジンの設定(`default.json`)を正として `editorSettings.json` を合わせ直す(`src/modules/keyboardmanager/KeyboardManagerEditorUI/Settings/SettingsManager.cs` の `CorrelateServiceAndEditorMappings`)
  - `editorSettings.json` は書き込まない。削除したリマップが新エディタ側でどう扱われるかは未確認
- 書き込みの安全策
  - 一時ファイルに書いてから置き換える(同じフォルダに一時ファイルを作り、rename で置換する)
  - 上書きする前の `default.json` を、本アプリのデータフォルダ(exe と同じフォルダの `backups\`。§1)に世代バックアップとして残す。既定は 10 世代。導入時のバックアップ(§4.7)とは別に管理する
  - 書き込みの間は `editorTransaction.lock` を排他的に開く(PowerToys の新エディタと同じ方式: `FileShare.None` で開き、取れなければ 50ms 間隔で再試行、10 秒でタイムアウト)。取れなければ保存せずにユーザーに知らせる
  - 読み込んだあとに `default.json` が他のプログラムに変更されていたら(読み込んだ時点の内容と比べて検知する)、上書きせずにユーザーに知らせる
- 反映の検証結果(2026-10-28、ユーザーの実環境、`useNewEditor: true`)
  - `default.json` を書き換えただけでは、エンジンは変更を反映しない(エンジンが起動中でも検知しない)
  - PowerToys の設定画面で Keyboard Manager をオフ→オンにすると、書き換えた内容が反映された
  - オフ→オンでは `default.json`・`editorSettings.json`・`settings.json` のどれも書き換えられなかった
  - 書き換えたあとに名前付きイベント `PowerToys_KeyboardManager_Event_Settings` をシグナルすると、オフ→オンなしで即座に反映された
- 反映のさせ方: 書き込みが成功したら、名前付きイベント `PowerToys_KeyboardManager_Event_Settings` をシグナルする。本アプリは既存のイベントを `OpenEventW` で開く(`CreateEventW` だと PowerToys が起動していなくても成功してしまい、反映されていないことを検知できないため)
  - 根拠: PowerToys 自身も保存後に同じことをしている(`src/modules/keyboardmanager/common/MappingConfiguration.cpp` の `SaveSettingsToFile` が `CreateEvent(nullptr, false, false, SettingsEventName)` → `SetEvent`)。エンジンはこのイベントを待ち受けて設定を読み直す(`KeyboardManagerEngineLibrary/KeyboardManager.cpp`)。イベント名は `common/KeyboardManagerConstants.h` の `SettingsEventName`
  - シグナルに失敗したとき(PowerToys が起動していない、など)は、保存は成功として扱い、反映されていない可能性をユーザーに知らせる
- 書き込むファイル名は `settings.json` の `activeConfiguration` に `.json` を付けたもの(既定 `default.json`)。PowerToys も `currentConfig + ".json"` に保存している

## 5. 未決定事項

- **UI の言語の切り替え方法**: OS の言語設定から自動で選ぶか、アプリ内の設定で切り替えるか。日本語の翻訳と CJK フォントの埋め込みをいつ行うか
- **JIS 配列への対応時期**と、配列を切り替える UI
- **対象とする操作の種類**: プログラムの起動、URI を開く、など新エディタの機能にどこまで対応するか
