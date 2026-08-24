# Rakukan 機能の選択移植計画

## 1. 目的と位置づけ

`karukan-feat-rakukan` は、Karukan を置き換える後継プロジェクトではない。
本流の開発は引き続き Karukan で行い、このリポジトリでは Rakukan の実装と運用実績から、
Linux・macOS 版 Karukan に有効な機能だけを選択して検証・移植する。

| リポジトリ | 役割 |
|---|---|
| `togatoga/karukan` | Karukan の上流リポジトリ |
| [`fukuyori/karukan-feat-rakukan`](https://github.com/fukuyori/karukan-feat-rakukan) | Rakukan 機能を選択移植する派生リポジトリ |
| `fukuyori/rakukan` | Windows 向け実装および移植元の参照リポジトリ |

この派生リポジトリで安定性と効果を確認できた変更は、内容に応じて本流 Karukan へ還元できる形に保つ。

計画作成時点（2026-08-24）の比較基準:

- Karukan: `2d7f4f8f03597ba4d714777b58e352f3814e7dc8`
- Rakukan: `de2c044cdc1b848a4bdea42b6750e2d73378e179`

## 2. 基本方針

1. **現行 Karukan を母体にする。** Rakukan の履歴やワークスペース全体はマージしない。
2. **機能単位で再実装または移植する。** 1つの変更で複数の機能を同時に導入しない。
3. **Karukan の既存設計を優先する。** chunk、候補ソース、学習TSV、KVキャッシュ、
   Linux・macOS共通Coreを維持する。
4. **OS固有実装を共通層へ持ち込まない。** Windows TSF、Named Pipe RPC、DLL ABI、
   WinUI、tray、engine-host は対象外とする。
5. **挙動変更には設定または段階導入を用意する。** 品質フィルタは観測モードから開始し、
   実測後に既定値を決める。
6. **移植元を記録する。** Rakukan のコードを実質的に利用した変更は、コミットメッセージと
   必要なソースコメントに対象ファイル・コミットを記載する。

## 3. ライセンスの扱い

Karukan は `MIT OR Apache-2.0`、Rakukan は MIT ライセンスである。
Rakukan のコードをそのまま、または実質的に移植する場合は MIT の条件を満たす必要がある。

- 移植元ファイルと Rakukan のコミットIDを記録する。
- 著作権表示・ライセンス表示が必要な規模の移植は `THIRD_PARTY_LICENSES` に追記する。
- Rakukan 由来部分を Apache-2.0 のみとして扱わない。
- アイデアだけを参考に Karukan の構造へ独自実装した場合も、由来が分かるようコミットに記録する。

## 4. 現状と重複機能

次の機能は現行 Karukan に既に導入されているため、Rakukan から再移植しない。

- Jinen v2 Q5モデルとモデルレジストリ
- tokenizer の byte-fallback 修正
- 高速beam searchとKVキャッシュ再利用
- 数字・英字・記号をAI推論から分離するchunk機構
- 数字の全半角、漢数字、大字、ローマ数字などの候補生成
- 候補ソースの識別とソース別表示
- 学習候補の削除
- ユーザー辞書とシステム辞書
- オフライン時のフォールバックとバックグラウンドモデルロード
- プリエディット内カーソル移動と手動chunk境界

Rakukan の `digits.rs`、`conv_cache.rs`、辞書クレートを丸ごと移植すると、これらの既存機能と競合する。
必要なテストケースや仕様だけを選び、Karukan の実装へ反映する。

## 5. 移植対象と優先順位

| 優先度 | 対象 | 効果 | 難易度 | 方針 |
|---|---|---|---|---|
| P0 | 異常候補の安全網 | 途中切れ、反復、過長候補を抑制 | 中 | 最優先で導入 |
| P0 | contextエコー対策 | 未変換文脈による変換崩壊を抑制 | 中 | 純粋関数として移植 |
| P1 | 学習履歴の整理 | 古い候補・誤学習の蓄積を抑制 | 中 | Karukan TSVを維持 |
| P1 | ユーザー辞書の再読込 | 再起動なしで編集を反映 | 中 | ディレクトリ監視を追加 |
| P2 | F6–F8変換 | ひらがな・全角カナ・半角カナ変換を追加 | 中 | 共通Coreで実装 |
| P2後半 | F9–F10変換 | 全角・半角英数変換を追加 | 高 | 生打鍵履歴の保持が前提（後述） |
| P3 | 範囲指定変換 | 読みの一部だけを変換・確定 | 高 | Karukan向けに再設計 |
| 任意 | CUDA/Vulkan | 対応環境で推論を高速化 | 高 | 単一バイナリ方式で実装 |
| 任意 | Jinen v2 F16 | 非量子化variantを選択可能にする | 低 | 既定モデルは変更しない |

## 6. フェーズ別計画

### Phase 0: 基準状態と検証材料の固定

目的は、以降の品質変更を比較できる基準を作ることである。

実施内容:

- 上流 Karukan の追従時点と Rakukan の参照コミットを文書またはコミットに記録する。
- greedy、beam、ライブ変換、辞書フォールバックの基準出力を保存する。
- 次の異常入力をモデル試験ケースとして用意する。
  - EOSに達しにくい長い読み
  - 同じ句を二重に出力しやすい読み
  - contextに同じ未変換かな文が含まれる読み
  - 数字、型番、英字を含む読み
  - 正当な反復表現（例: `わかったわかった`）
- `cargo test --workspace --locked` が通る状態を基準とする。
- karukan-engine の統合テストは初回実行時にモデルを自動ダウンロードするため、
  オフライン環境で workspace テストを完走させるにはテストの feature 分離が
  先に必要になる可能性がある。必要ならこの分離を Phase 0 の作業項目とする。

完了条件:

- 基準テストが再現可能である。
- モデルが無い環境でも通常の単体テストが完走する。
- 作業開始時点の上流コミットが記録されている。

### Phase 1: 変換品質の安全網

#### 1-A. 読み長に応じた生成予算

固定の `max_new_tokens` だけに依存せず、読みの文字数から最低生成予算を計算する。
上限を設け、異常入力で無制限に推論時間が伸びないようにする。

候補式:

```text
max(configured_max, reading_chars * 2 + 8), capped at 256
```

実際の係数と上限はJinen v1/v2の試験結果で決定する。

#### 1-B. EOS未到達候補の扱い

- beam searchではEOSへ到達した候補と、生成上限に達した未完了候補を区別する。
- 未完了候補を通常候補へ混在させない。
- 完了候補が0件の場合は、読み・辞書候補へフォールバックする。全滅時に読みへ戻す
  フォールバック自体は既に2段ある（`KanaKanjiConverter::convert` と
  `convert_chunk`）ので、これを壊さないことが条件になる。
- 現状の実装は `finalize_beams`（`kanji/llamacpp.rs`）がEOS到達beamと打ち切りbeamを
  1つのリストへ連結してスコア順に返し、`BeamState` に完了フラグがないため、
  beam内部と戻り値の型変更が必要になる。
- この型変更は 1-E のスコア伝搬と同一箇所に入る。1-B の時点で完了フラグとスコアを
  一度に運ぶ型（`Vec<(String, f32, bool)>` 相当）を導入し、1-E では配管を再利用する。
- Karukan のKVキャッシュ再利用方式は維持し、Rakukan のbeam実装全体は移植しない。

#### 1-C. 退化候補フィルタ

モデル出力後に次を検査する純粋関数を `karukan-engine` に追加する。

- 空文字列
- 読みに比べて極端に短い候補
- `reading_chars * 1.5 + 2` を超える候補
- 一定長以上の同一部分列を連続出力する候補
- 読み自身が反復している場合の誤検出回避
- 読みの途中だけをかなで返すprefix-echo候補
- 括弧内がかなだけの不要なルビ状出力

長さや反復の閾値は定数にまとめ、モデル非依存の単体テストを付ける。

既存のフィルタは `KanaKanjiConverter::convert` 内の trim・空文字除去・完全一致dedupのみで、
長さ・反復・prefix-echo検査は存在しない。追加する検査は、beam結果のトークンとスコアが
まだ手元にある `convert` のbeam分岐、または純粋な文字列規則なら `clean_model_output` に置く。

LRU変換キャッシュとの位置関係を明示的に決める:

- キャッシュ格納の**前**にフィルタすると、退化候補は再推論されず永続的に消えるが、
  閾値の設定変更が既存エントリへ反映されない。
- キャッシュ格納の**後**にフィルタすると、設定変更が即反映されるが、退化出力も
  キャッシュに残る。
- 初期実装ではエンジン側（格納前）に置き、閾値は定数とする。設定化する場合に再検討する。

#### 1-D. contextエコー対策

入力の先頭と一致する長いかなrunを含む未変換文がcontextにある場合、
該当する文だけを変換contextから除外する。

- context全体を切り捨てず、句点等で区切った該当文だけを除外する。
- 短い読みには適用しない。
- 漢字混じりの正しいcontextを除外しない。
- 発動回数をdebugまたはtraceログで観測できるようにする。
- contextとして保持している原文自体は変更しない。

補足（現行コードで確認済みの発生経路と副作用）:

- エコーを能動的に作る経路が実在する: chunkのモデル変換が空振りすると
  `convert_chunk` が生かな読みへフォールバックし、それが `preceding_converted` に
  連結されて次chunkのlctxへそのまま入る（`chunk/mod.rs` → `lctx_for`）。
  この経路を対策の対象に含める。
- lctxはLRU変換キャッシュのキーの一部なので、除外の発動が入力途中で揺れると
  キャッシュヒット率を下げる。除外判定は同じ入力に対して安定であること。

#### 1-E. confidenceフィルタ

beam search内部の累積log-probを生成トークン数で正規化して利用する。

ただし「既に返している」は内部までで、スコアはAPI境界で捨てられている:
`KanaKanjiConverter::convert` は `Vec<String>` を返し、beam結果のスコアは
`backend.rs` の `for (output_tokens, _score)` で破棄され、IM側のLRUキャッシュも
文字列リストのみ保持する。したがってconfidenceフィルタには
Backend戻り値型 → convert API → キャッシュエントリ → AnnotatedCandidate を通す
スコア配管が必要で、これは 1-B の完了フラグと同一の型変更である。
配管は 1-B で導入した型に相乗りし、1-E 単独での型変更は行わない。

1. 最初は候補ごとの平均log-probをログへ出すだけにする。
2. 実データを収集後、最良候補との差が大きい候補を除外する設定を追加する。
3. 最良候補自体を棄却する絶対閾値は既定で無効にする。

想定設定:

```toml
[conversion]
confidence_margin = 3.0
# min_top_confidence = -4.0
```

完了条件:

- 未完了beam、過長候補、反復候補がテストで除外される。
- 正当な反復入力は除外されない。
- contextエコー対策で前後の無関係な文が保持される。
- 既存の高速beam searchの性能を大きく後退させない。
- フィルタで候補が全滅した場合に読み・辞書候補へ安全に戻る。

### Phase 2: 学習履歴とユーザー辞書

#### 2-A. stale学習の削除

Karukan のTSV形式を維持したまま、読込時に古いエントリを整理する。

想定設定:

```toml
[learning]
stale_days = 180
```

- `0` または未指定時の意味を明確にする。
- 期限判定は `last_access` を使う。
- 削除後はキャッシュをdirtyとして、次回保存時にTSVへ反映する。
- 既存の `max_entries` とスコアリングは維持する。

#### 2-B. 候補ソースに基づく学習方針

`CandidateSource` を利用し、何を学習するかを明示する。

初期方針:

| ソース | 学習 |
|---|---|
| UserDictionary | する |
| Learning | 更新する |
| Model | 明示選択時にする |
| Dictionary | する |
| Rewriter | 明示選択時にする |
| Fallback | しない |

現状の `record_learning` が持つ条件はEmojiモード除外と `max_surface_chars` の2つだけで、
`surface != reading` チェックは存在しない（かなをそのまま確定しても
`reading → reading` が学習されている）。

かな確定の扱い（2026-08-24 決定）: かな確定（`surface == reading`）は mozc と同様に
**学習に含める**。よく使うかな語を上位に押し上げる働きを残すためで、
`surface != reading` の除外条件は導入しない。学習の選別は候補ソースのみで行い、
誤学習対策は Fallback ソースの除外が担う（モデル空振り時の生かなは
Fallback ソースなので、この方針でも学習されない）。

また `record_learning` は `(reading, surface)` しか受け取らず `CandidateSource` を
参照していないため、ソース別方針にはコミット経路3箇所
（`conversion.rs` の `finish_conversion`、`input.rs` の `commit_composing`、
`mod.rs` の `commit`）へsourceを渡す配線を新設する。

ライブ変換を候補ウィンドウなしで確定した場合は、ソース別方針と
最大文字数・入力モードの既存条件を合わせて判定する。

#### 2-C. ユーザー辞書のhot reload

Karukan は複数のMozc TSV/KRKNファイルをディレクトリから読み込むため、
Rakukan の単一TOMLファイル監視はそのまま使わない。

- ディレクトリ内の対象ファイルについて、パス・更新時刻・サイズからfingerprintを作る。
- 毎打鍵で全ファイルを読み直さず、一定間隔または辞書参照時に変更だけを確認する。
- 読込に失敗した場合は最後に成功した辞書を維持する。
- ファイルの追加・更新・削除をすべて検出する。
- 再読込中もシステム辞書とモデル変換を利用可能にする。

完了条件:

- 期限切れ学習が読込時に整理される。
- Fallbackを確定しても学習履歴を汚さない。
- ユーザー辞書の追加・編集・削除がIME再起動なしで反映される。
- 壊れた辞書ファイルを置いても、直前の正常な辞書が利用できる。

### Phase 3: F6–F10変換

Rakukan のTSFハンドラは移植せず、Karukan の共通Coreにキー操作として実装する。

| キー | 動作 |
|---|---|
| F6 | ひらがな |
| F7 | 全角カタカナ |
| F8 | 半角カタカナ |
| F9 | 全角英数。小文字・大文字・先頭大文字を巡回 |
| F10 | 半角英数。小文字・大文字・先頭大文字を巡回 |

キー伝達は両フロントエンドで既に完了している: fcitx5 addonは全キーをverbatimで
Coreへ転送し、macOSの `KeyCodeMap.swift` はF1–F12 → 0xffbe–0xffc9 のマップを
既に持つ。必要なのはCore側のkeysym定数追加とハンドラのみで、
フロントエンドの変更は不要である。

本フェーズは難易度の異なる2段に分割する。

#### 3-A. F6–F8（かな系）

F6–F8の変換先はいずれも現在の表示文字（かな）から導出できる。
`kana.rs` のひらがな・カタカナ・半角カタカナ変換をそのまま使う。

- Coreのキー識別へF6–F10のkeysym定数（0xffc3–0xffc7）を追加する。
- 変換中、ライブ変換中、通常プリエディットの各状態で同じ結果になるようにする。
- 既存のkana変換、width変換、rewriterを再利用する。

#### 3-B. F9–F10（英数系）

F9/F10は打鍵したローマ字列への変換であり、難易度は「中」ではなく「高」である。
`InputBuffer` は発火した打鍵を表示文字 `Converted(かな)` に再記録して
元のASCII打鍵列を破棄する設計のため、現状の構造からローマ字列は復元できない
（「わたし」から `watasi` は取り出せない）。

- 生打鍵履歴の保持（mozcのcomposerが持つraw input相当）を `InputBuffer` または
  並走する記録として設計する。中核データ構造への変更なので、3-Aとは独立に
  設計・レビューする。
- 同じキーの再押下による巡回状態を、入力が変更されたらリセットする。
- 巡回状態はF9/F10間で共有するかを含め、mozcの挙動を基準に決める。

完了条件:

- LinuxとmacOSで共通のCoreテストが通る。
- F6–F10後にEnterした文字列が画面表示と一致する。
- F9/F10の巡回で記号と英字の幅が意図せず混在しない。
- 生打鍵履歴の追加が既存のromaji評価（`evaluate_run`）と
  Backspace/カーソル編集の等価性を壊さない。

### Phase 4: 範囲指定変換

Rakukan の `RangeSelect` はWindows TSFのcomposition管理に結合しているため、
概念だけを利用してKarukan向けに設計する。

初期仕様:

- `Shift+Right` で読みの先頭から選択範囲を伸ばす。
- `Shift+Left` で選択範囲を縮める。
- Spaceで選択範囲だけを変換する。
- Enterで選択範囲を確定し、残りの読みでライブ変換を再開する。
- Escapeで範囲指定を解除し、元の読みまたは表示へ戻す。
- 数字・記号chunkの途中に不正な境界を作らない。

設計方針（2026-08-24 詳細化）: Rakukan の失敗はプラットフォーム
（TSF がcompositionを所有し、範囲・編集が非同期 edit session 越しになる）
との戦いが原因であり、Karukan ではフロントエンドが preedit の
レンダラに過ぎないため、範囲指定は純粋なエンジン内状態設計の問題に
還元される。失敗リスクは範囲の表現を極限まで単純にすることで下げる。

Core 変更（設計済み）:

- 範囲は「読みの先頭からの文字数」**1個の整数**に限定する
  （`range_select: Option<usize>` フィールド。新しい状態 enum は作らない）。
  anchor は常に読みの先頭で、任意区間・anchor 移動は将来の拡張に分離する。
- 範囲の解除は `end_composition` / `clear_composition` という既存の
  全出口1本道で行い、Escape・Backspace・フォーカス移動での状態破損を防ぐ。
- 範囲モード中のその他の編集キーは「範囲解除 → 通常処理」。Ctrl+J も同様
  （範囲変換は chunk 境界と独立）。ライブ変換表示中の Shift+Right は、
  先に生読み表示へ戻してから範囲モードに入る（選択対象は常に読み）。
- 表示は定義済み未使用の `AttributeType::UnderlineDouble` と
  `Preedit::from_segments` を使う。
- 範囲の変換は `reading[..n]` を既存の `build_conversion_candidates` →
  `enter_conversion_state` に渡すだけ。変換キャッシュは（読み, lctx）キーで
  そのまま効き、数字・記号境界は prefix の変換自体が chunk 分割を通る
  ため無害化される。Conversion 状態に `span: Option<usize>` を追加する
  （None = 従来の全体変換で、既存経路は不変）。
- 新設が必要な唯一のプリミティブは**部分確定** `commit_range(n, text)`:
  Commit(text) 発行 → InputBuffer 先頭 n 要素の drain（raw 打鍵・
  chunk_breaks も同時にシフト）→ 学習記録（ソース方針適用）→
  `refresh_input_state()` で残り読みのライブ変換を再開。Core の1操作として
  定義し、fcitx5/macOS 固有コードへ分散させない。

コミット単位: (1) `InputBuffer::drain_prefix` + テスト、(2) `range_select` +
Shift+←→ + 描画、(3) prefix 変換 + `span` 付き Conversion、
(4) `commit_range` + 学習 + ライブ再開、(5) 実機確認 + docs。

最大のリスクは「Commit と preedit 更新を1キーイベントで両立」する
部分確定のフロントエンド適用で、実アプリでしか検証できない。問題が出た
場合は部分確定を2段階（確定のみ → 次イベントで preedit 復元）へ逃がす。

利用できる既存部品（確認済み）:

- `PreeditAttribute` は任意のchar範囲を持て、選択セグメント用の
  `AttributeType::UnderlineDouble` と `Preedit::from_segments` が定義済み未使用で
  存在する。preedit表現側の追加実装はほぼ不要。
- Shift+Left/Right は現状caret移動に折り畳まれている（`input.rs` がarrowで
  shiftを無視）ため、キー割当は空いている。`shift_active` は既に
  `process_key_composing` まで配線済み。
- 難易度「高」の主因は状態機械へのanchor追加と部分確定オペレーションの新設であり、
  ここに設計工数を集中する。

完了条件:

- ひらがな、ライブ変換結果、通常変換候補の各状態から範囲指定へ移行できる。
- 選択範囲の確定後も残りの読みが失われない（ライブ変換が再開する）。
- Escape、Backspace、フォーカス移動で状態が破損しない。
- LinuxとmacOSで同じ状態遷移テストを共有できる。
- 部分確定が実アプリ（端末・ブラウザ・エディタ）で正しく挿入される。

### Phase 5: 任意機能

#### 5-A. CUDA/Vulkan

Rakukan の複数DLLと動的ABIはWindows配布用なので導入しない。
Karukanでは次の単純な構成を検討する。

- `karukan-engine` に `cuda` / `vulkan` Cargo featureを追加する。
- `n_gpu_layers` と `main_gpu` を変換設定へ追加する。
- CPUビルドを既定として維持する。
- 利用不能な設定では明確なエラーを出し、可能ならCPUへフォールバックする。
- GPT-2系とQwen3系でGPU利用可否を分けて検証する。

GPU対応はビルド時間、配布物、ドライバ依存を増やすため、通常の品質改修と別ブランチで扱う。

#### 5-B. Jinen v2 F16 variant

- `jinen-v2-xsmall-f16` と `jinen-v2-small-f16` をモデルレジストリへ追加する。
- Q5を既定のまま維持する。
- ダウンロードサイズを設定資料へ明記する。
- Q5との差を精度・速度・メモリ使用量で計測する。

前提条件（先に対応が必要）:

- `prefetch_all_models` は登録済み全variantをダウンロードし、macOSの
  `make install` がこれを呼ぶ。F16をregistryへ足すとインストール時の
  ダウンロードが数GB増えるため、prefetch対象の絞り込み
  （既定モデルのみ、または明示指定分のみ）を先に入れる。
- `model_config.rs` のテストがvariant数=5をハードアサートしているため、
  variant追加時にテストも更新する。

## 7. 移植しないもの

次のRakukan固有機能は、この派生リポジトリの対象外とする。

- Windows TSFフロントエンド
- Named Pipe RPCとpostcardプロトコル
- engine-hostプロセス
- CPU/CUDA/Vulkan DLLの動的ローダーとABI
- WinUI設定アプリとtray
- Windowsインストーラー
- WMI、レジストリ、Windows mutexを使う検出・監視
- Rakukanのセッション状態機械全体
- Rakukanの辞書バイナリ形式とbincode学習ファイル
- `RakunEngine` 全体

将来Windows版Karukanを同じリポジトリへ統合する方針が決まった場合は、別計画として再評価する。

## 8. ブランチと上流追従

想定ブランチ:

| ブランチ | 用途 |
|---|---|
| `main` | 検証済みの派生版 |
| `feat/rakukan-conversion-safety` | Phase 1 |
| `feat/rakukan-learning` | Phase 2 |
| `feat/rakukan-fkeys` | Phase 3 |
| `feat/rakukan-range-select` | Phase 4 |
| `feat/rakukan-gpu` | 任意GPU対応 |

上流追従方針:

1. 公開済みの派生`main`はrebaseせず、`upstream/main`を定期的にmergeする。
2. 作業中のfeatureブランチは、取り込み前に最新の派生`main`へrebaseしてよい。
3. 上流同期とRakukan機能移植を同じコミットに混ぜない。
4. 競合解決後はworkspace全体のテストを実行する。
5. Rakukanリポジトリは参照元として扱い、異なる履歴を直接mergeしない。

推奨remote名:

```text
origin    https://github.com/fukuyori/karukan-feat-rakukan.git
upstream  https://github.com/togatoga/karukan.git
rakukan   https://github.com/fukuyori/rakukan.git
```

## 9. 検証方針

各フェーズで最低限、次を実行する。

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
```

変更範囲に応じて追加する。

- `cargo test -p karukan-engine`
- `cargo test -p karukan-im`
- fcitx5統合テスト
- macOS Swiftテスト（macOS環境）
- Jinen v1/v2、greedy/beamの実モデル試験
- モデル未導入・オフライン環境でのフォールバック試験
- 長文入力、数字、英字、記号、カーソル編集の回帰試験

性能変更では、同じモデル・同じスレッド数・同じ入力セットで次を比較する。

- greedy変換時間
- beam変換時間
- 1打鍵あたりのライブ変換時間
- 候補全滅率
- フィルタ発動率
- 最大メモリ使用量

## 10. コミット単位と完了条件

1コミットは、原則として次のいずれか1つに限定する。

- モデル非依存の純粋関数と単体テスト
- 設定項目と設定テスト
- 変換経路への接続
- フロントエンドのキー伝達
- ドキュメント

各コミットに以下を記載する。

- 変更目的
- Karukan側の変更箇所
- Rakukan由来の場合は参照コミット・ファイル
- 既定動作への影響
- 実行したテスト

各フェーズは次を満たした時点で完了とする。

- workspaceテストが通る。
- 新規仕様の単体テストがある。
- 設定追加時は設定資料と既定設定が更新されている。
- Linux/macOS共通機能はCoreに実装されている。
- 既知の回帰と未決事項が文書化されている。
- 問題発生時にフェーズ単位でrevertできる。

## 11. 実施順序

```text
Phase 0 基準固定
  → Phase 1 変換品質の安全網
  → Phase 2 学習・辞書
  → Phase 3-A F6–F8
  → Phase 3-B F9–F10（生打鍵履歴の設計後）
  → Phase 4 範囲指定変換
  → Phase 5 GPU / F16（必要な場合のみ）
```

最初の実装対象は Phase 1-A～1-D とする。1-B ではEOSフラグとスコアを一度に運ぶ
型変更を入れ、1-E はその配管を再利用する。confidenceフィルタの強制適用、
Phase 3-B、範囲指定変換、GPU対応は、先行フェーズの検証結果と独立させる。
Phase 3-B は中核データ構造（生打鍵履歴）の設計を伴うため、
Phase 4 と順序を入れ替えてもよい。
