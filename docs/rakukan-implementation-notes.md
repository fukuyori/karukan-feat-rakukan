# Karukan からの変更点解説

本リポジトリ(karukan-feat-rakukan)が上流 [togatoga/karukan](https://github.com/togatoga/karukan)
からどこをどう修正したかの解説。各節を「Karukan の従来動作 → 本リポジトリでの
修正 → なぜそうしたか → 実装のポイント → 挙動の違い」の順で書いている。
比較基準は上流 `2d7f4f8`(2026-08-24 時点)。

計画と検証条件は [rakukan-porting-plan.md](rakukan-porting-plan.md)、
変更前後の出力比較の手順は [baselines/README.md](baselines/README.md) を参照。

---

## 1. モデル依存テストの扱い

**Karukan では**: `llamacpp.rs` の `beam_search_tests` だけが「モデルを解決
できなければスキップ」の作りで、`backend.rs` のユニットテスト3件・
`kanji_conversion_tests.rs` の12件・`romaji_tests.rs` の1件は `.expect()` で
失敗していた。そのためオフライン + HF キャッシュ空の環境では
`cargo test --workspace` が完走しなかった。

**本リポジトリでは**: 取り残されていた16箇所を既存のスキップ規約に揃えた。

```rust
let Some(model) = load_model() else {
    eprintln!("model unavailable, skipping");
    return;
};
```

モデルがキャッシュ済みの環境(CI 含む)では従来どおり全テストが実行されるので、
テストの守備範囲は変わらない。変わるのはモデル無し環境の結果が
「失敗」から「スキップして完走」になることだけ。

モデル無し環境は「空の `HF_HOME`」+「到達不能な `HF_ENDPOINT`
(例: `http://127.0.0.1:1`、即 connection refused になる)」で再現できる。

**変更ファイル**: `karukan-engine/src/kanji/backend.rs`(tests)、
`karukan-engine/tests/kanji_conversion_tests.rs`、`karukan-engine/tests/romaji_tests.rs`

---

## 2. 生成トークン数の上限 — 固定値から読み長依存へ

**Karukan では**: モデルの生成上限は `ConversionConfig::max_new_tokens` = 固定50
トークンだけで、全呼び出しが既定値を使っていた。読みが21文字を超えると正当な
変換でも出力が途中で切れうる。逆にこの値を単純に増やすと、EOS に到達しない
異常入力で推論時間が読み長と無関係に伸びる。

**本リポジトリでは**: 予算を読み長から計算するようにした
(`karukan-engine/src/kanji/backend.rs`)。

```rust
pub fn generation_budget(reading_chars: usize, configured_max: usize) -> usize {
    (reading_chars.saturating_mul(2).saturating_add(8))
        .max(configured_max)      // 既定50は floor として残る
        .min(MAX_GENERATION_BUDGET) // 256
}
```

- 係数2の根拠: 漢字出力はおおむね読み1文字 ≦ 1トークン、byte-fallback の
  最悪ケースで1文字3トークン。2倍 + スラック8で実在の変換を覆う
- 短い読み(21文字以下)では従来の50がそのまま効くので挙動不変
- 上限256は異常入力の時間制限

**併せて直した箇所**: greedy 経路(`generate_with_sampler`)は beam と違い
モデル既定の固定 `n_ctx`(256セル)のコンテキストを使うため、予算を上げると
「プロンプト + 生成 > n_ctx」で decode が失敗しうる。そこで最終生成位置が
n_ctx を超えないクランプを入れた。beam search は
`n_cells = input_len + 2 × beam × (max_new + 1) + headroom` と自前でセルを
確保する作りなので、この問題を元々持っていない。

**挙動の違い**: EOS に到達する通常の変換は完全に不変(基準13ケースの
greedy / beam 出力が一致することを確認済み)。読み21文字超で生成上限だけが伸びる。

---

## 3. beam search の戻り値 — EOS 到達と打ち切りの区別

**Karukan では**: beam search の `finalize_beams` が、EOS に到達した beam と
生成上限で打ち切られた beam を**1つのリストに連結してスコア順に返していた**。
戻り値は `Vec<(Vec<LlamaToken>, f32)>` で完了フラグを持たず、呼び出し側は
途中で切れた文章を通常候補と区別できない。さらに累積 log-prob スコアは
`KanaKanjiConverter::convert` の `for (output_tokens, _score)` で捨てられ、
API の外に出ていなかった。

**本リポジトリでは**: 戻り値を事実を運ぶ型に変えた。

```rust
pub struct BeamCandidate {
    pub tokens: Vec<LlamaToken>,
    pub score: f32,     // 累積 log-prob。従来は境界で破棄されていた
    pub finished: bool, // EOS 到達か、生成上限による打ち切りか
}
```

- `generate_beam_search` と参照実装 `generate_beam_search_full_eval` の両方を
  変更し、fast/reference の等価性テストは finished フラグも含めて比較する
- **切り捨てるかどうかの判断は llamacpp 層に置かない**: `convert` の beam 経路で
  `finished` により分割し、打ち切り beam は数を debug ログに出して候補から外す。
  途中で切れた文章は読みの変換ではない、というのがポリシーの根拠
- 全 beam が打ち切りの場合は、Karukan に元からある「候補ゼロなら読みを push」
  フォールバックがそのまま効く(この安全網は壊していない)
- `score` を運ぶようにしたのは §6 の confidence 観測のため。型変更を2回
  やらないよう、フラグとスコアを一度に導入した

**挙動の違い**: §2 の予算スケールにより通常の変換で打ち切りはほぼ発生しない
ため、実用上の出力は不変(基準13ケース一致)。異常入力での途中切れ候補だけが
消える。呼び出し側(`karukan-cli/src/bin/server.rs` の2箇所、テスト、ベンチ)は
機械的に追従した。

---

## 4. モデル出力の検査 — trim だけから退化候補フィルタへ

**Karukan では**: モデル出力の検査は trim・空文字除去・完全一致 dedup のみ
(`clean_model_output` は `text.trim()` だけ)。小さいモデルが出す「変換に
なっていない出力」— 入力かなのエコー、暴走反復、極端な長短、ルビ状出力 —
がそのまま候補に並んでいた。

**本リポジトリでは**: モデル非依存の分類器を新設し
(`karukan-engine/src/kanji/quality.rs`)、`convert` の greedy / beam 両経路で
候補追加前に検査するようにした。

```rust
pub enum DegenerateReason { Empty, TooShort, TooLong, Repetition, PrefixEcho, RubyEcho }
pub fn degenerate_reason(candidate: &str, reading: &str) -> Option<DegenerateReason>
```

閾値は保守側に倒してある。「悪い候補を時々通す」フィルタは鬱陶しいだけだが、
「正しい変換を食う」フィルタはバグ、という設計判断による。

| 理由 | 条件 | 誤検出を防ぐ側の仕様 |
|---|---|---|
| `Empty` | 空・空白のみ | — |
| `RubyEcho` | 括弧(半角/全角)の中身が**かなだけ**(明日(あした)) | 「株式会社(仮)」のようにかな以外を含む括弧書きは通す |
| `PrefixEcho` | かなのみ、かつ読みの**真の前方一致**(カタカナはひらがな正規化して比較) | 読み**全体**と等しいかな候補は正当(かな語がそれ自身に変換されるケース)なので通す |
| `TooLong` | 候補 > 読み × 1.5 + 2 文字 | 漢字出力は通常読みより**短い**ので暴走にしか当たらない |
| `TooShort` | 候補 × 4 < 読み(読み8文字以上のときのみ) | 短い読みは正当に強く圧縮する(こころざし→志は5:1)ため8文字未満は不適用 |
| `Repetition` | 長さ2以上の単位の隣接反復が候補にあり、**読みには無い** | 読み自身が反復するなら検査ごと停止(わかったわかった。とうきょうとっきょきょかきょくも「きょきょ」が隣接反復)。単位1の反復(会社社長)は日常語なので対象外 |

**判定順序に意味がある**: エコー系を長さ系より先に判定する。ルビ状出力は読み
より長く、かなエコーは短いので、順序を逆にすると理由が TooLong / TooShort に
化けてログの診断価値が落ちる。

除外は理由付き debug ログに残り、全滅時は読みフォールバックに落ちる。検査は
LRU 変換キャッシュより**内側**(格納前)なので、退化候補は再推論されず消えた
ままになる — 閾値を設定化するならこの位置は再検討が要る。

**テスト**: モデル非依存の単体11件。「正当なものを通す」側のテストを
「悪いものを落とす」側と同数以上入れてある。

---

## 5. 変換コンテキスト — 無加工から入力エコー文の除外へ

**Karukan では**: 変換コンテキスト(lctx)は組み立てたまま無加工でモデルに
渡っていた。ところが lctx に**未変換のかな文**が混じると、モデルは変換せず
かなをエコーしやすい。しかもエコーを能動的に作る経路が Karukan 自身にある:
chunk のモデル変換が空振りすると `convert_chunk` が生かな読みへフォールバック
し、それが `preceding_converted` に連結されて**次 chunk の lctx にそのまま入る**。
基準ケース echo-02(context「あしたはあめかもしれないとおもった」+ 読み
「あしたはあめ」)で、greedy が生かな「あしたはあめ」を返すことを実測記録した。

**本リポジトリでは**: プロンプト構築の直前に、context から入力のエコー文だけを
除外するようにした(`quality.rs::echo_free_context`)。

- context を文単位(。．！？!? と改行)に分割し、**読みの先頭かな5文字を含む
  漢字なしの文だけ**を除去。他の文は原文のまま
- 読みのかなが5文字未満なら何もしない(短い読みは偶然一致しやすい)
- **漢字を含む文は絶対に除去しない** — それは変換済みテキストであり、かなが
  読みと重なっていてもモデルに残すべきコンテキストそのもの

**適用位置が要**: エンジンの `convert` 内、つまり IM 側の変換キャッシュの
**内側**で適用する。これにより (1) IM のキャッシュキー(生の lctx)が揺れず
ヒット率に影響しない、(2) 呼び出し側が保持する context 原文は書き換わらない、
(3) IM の chunk 変換にも CLI サーバーにも同じ対策が効く。発動は debug ログで
観測できる。

**挙動の違い(実測)**: echo-02 の greedy が「あしたはあめ」→「明日は雨」に
改善。echo-01 の beam からかなエコー候補が消滅。対照例 echo-03(漢字混じり
context)と他の10ケースは完全一致。

---

## 6. beam 候補の confidence — 破棄から観測へ

**Karukan では**: beam の累積 log-prob は API 境界で捨てられ、候補の確信度は
どこからも見えなかった。

**本リポジトリでは**: §3 でスコアが境界を越えるようになったのを使い、候補ごとの
平均 log-prob(`score / 生成トークン数`)を debug ログに出す。**除外はまだ
行わない** — 実データを集めてから `confidence_margin`(最良候補との差)による
除外を検討する段階設計で、最良候補自体を棄却する絶対閾値は既定で無効にする
方針にしている。

---

## 7. 学習キャッシュ — 出口の追加(`stale_days`)

**Karukan では**: 学習キャッシュ(TSV)は読み込み時に一切掃除されない。
エントリが消えるのは手動削除(Ctrl+Backspace)か、保存時の `max_entries`
(既定10,000)超過だけ。TSV には `last_access`(Unix 秒)列が**元からある**のに
時間ベースの整理には使われていなかった。

**本リポジトリでは**: 読み込み時の期限切れ整理を足した
(`karukan-engine/src/learning.rs`)。

- `LearningConfig` / `[learning]` 設定に `stale_days: u32` を追加。
  **0 = 無期限で、これが既定** — Karukan の従来動作が既定のまま残る。
  挙動変更は opt-in という方針(スコア式が古いエントリを既に強く沈めている
  ため、削除は掃除であって品質改善の主役ではない)
- `load` の末尾で `last_access < now − stale_days × 86400` のエントリを削除。
  **境界は保持**(経過 == stale_days は生き残る)
- 削除があった場合だけ dirty にし、次回保存で TSV へ反映

---

## 8. 学習の記録条件 — 無選別からソース別へ

**Karukan では**: `record_learning(reading, surface)` の条件は Emoji モード
除外と `max_surface_chars`(50文字)だけで、候補の出所(`CandidateSource`)は
見ていなかった。つまり:

1. モデルが変換を出せなかったときの**読みフォールバック候補**を確定すると、
   「変換が無かったこと」がユーザーの選択として学習される
2. 候補ウィンドウを開かない**ライブ変換の確定**で、モデルの第一候補が誤って
   いてもそのまま学習され、次回から最上位に来る(自己強化)

**本リポジトリでは**: 候補ソースと「明示選択かどうか」で記録を選別するように
した(`karukan-im/core/src/core/candidate.rs`)。

```rust
pub fn records_learning(&self, explicit: bool) -> bool {
    match self {
        Fallback => false,                          // 常に学習しない
        Model | Rewriter => explicit,               // 明示選択時のみ
        Learning | UserDictionary | Dictionary => true,
    }
}
```

`explicit` は「候補ウィンドウで自分が選んだ確定」(true)か「表示中のものを
そのまま受け入れた確定 = ライブ変換の Enter」(false)か。

**かな確定の扱い**: かな確定(`surface == reading`、ソースなし)は mozc と
同様**学習に残した**。よく使うかな語を上位に押し上げる働きを消さないため。
モデル空振り時の生かなは Fallback **ソース**として除外されるので、この方針でも
汚染は起きない。

**配線**: 確定経路は3つ(変換確定 / Composing 確定 / フォーカスアウト)あり、
Karukan では `(reading, surface)` しか流れていなかったところに、
すべてソース・学習可否を通した。

**挙動の違い**: 読みフォールバック確定と、候補ウィンドウなしのライブ変換確定が
学習されなくなる。候補を自分で選んだ確定・かな/カタカナ確定は従来どおり。

---

## 9. ユーザー辞書 — 起動時一回読みから hot reload へ

**Karukan では**: `user_dicts/` は IME 初期化時に一回だけスキャンされ
(`init_user_dictionaries` は2回目以降早期 return)、辞書ファイルを編集しても
IME を再起動するまで反映されなかった。

**本リポジトリでは**: fingerprint 監視の `UserDictWatcher` を新設し
(`karukan-im/core/src/core/engine/user_dicts.rs`)、初期化も再読込もこれに
一本化した。

- ファイルごとに(パス, mtime, サイズ)の stamp を取り、**変化したファイル
  だけ**再パースする。チェック1回のコストはファイル数回の stat のみ
- **ファイル単位の last-good キャッシュ**: 各ファイルについて「最後の読込試行
  時の stamp」と「最後に成功した読込のパース済み辞書」を持つ。壊れた編集は
  直前の正常な内容を使い続け、stamp は進める — ファイルが次に変わるまで
  再試行しないので警告ログの連打も起きない
- `BTreeMap` でパス順を保ち、Karukan の一回読みが持っていたアルファベット順の
  マージ優先順位をそのまま維持
- 存在しないディレクトリは空として扱う — 後からディレクトリを作っても検出される
- `process_key` 冒頭(モデルロードの poll の隣)から **2秒スロットル**で確認。
  打鍵毎のコストはタイムスタンプ比較1回

**必要だったリファクタ**: `Dictionary::merge` は `Vec<Dictionary>` を消費する
形だったが、`Dictionary` は `Clone` を持たずファイル単位キャッシュと両立しない。
`IntoIterator<Item = &Dictionary>` を受けてエントリを clone する形に変更した
(挙動不変、呼び出し3箇所を機械的更新)。

---

## 10. F6/F7/F8 — かな系の固定変換を追加

**Karukan では**: F キーは未割当だった(Composing ではアプリに透過、Conversion
では no-op)。カタカナへの変換手段は Ctrl+K のモード切替のみで、mozc 的な
「入力全体をひらがな / 全角カタカナ / 半角カタカナに変換」する操作はなかった。
`keycode.rs` に F キーの keysym 定数もなかった。

**本リポジトリでは**: F6(ひらがな)/ F7(全角カタカナ)/ F8(半角カタカナ)を
追加した(`karukan-im/core/src/core/engine/fkeys.rs`)。

**実装の要は「新しい状態を作らない」こと**: 3つの変換を候補に持つ**通常の
Conversion 状態**として実装した。押した F キーの変換が選択された状態で
`enter_conversion_state` に入るだけなので、F キー同士の切替は選択の移動、
Enter は通常の確定経路(かな確定として学習)、Escape は編集可能な composing
への復帰 — すべて既存機構がそのまま働く。

- 読みは Space 変換と同じ `settled_reading`(「あいk」→「アイk」)
- 変換先は `kana.rs` の既存関数(`hiragana_to_katakana` /
  `katakana_to_half_width`)で表示かなから導出
- 未入力時はアプリへ透過、絵文字モードでは no-op

**フロントエンドの変更はゼロ**: fcitx5 addon は元から全キーを verbatim で
core へ転送しており、macOS の `KeyCodeMap.swift` も F1–F12 → keysym のマップを
既に持っていた。必要だったのは core の keysym 定数とディスパッチだけ。

---

## 11. 入力バッファ — 生打鍵の破棄から保持へ

**Karukan では**: `InputBuffer` は表示1文字 = 1要素の配列で、ローマ字打鍵は
ルールが発火すると表示文字 `Converted(かな)` に**再記録され、元の ASCII 打鍵列は
捨てられる**。この設計は表示との1対1対応という美点を持つ一方、「わたし」から
`watasi`(打った通りのキー)を復元する手段がなかった。F9/F10(§12)は
mozc 的には打鍵そのものへの変換なので、この情報が必須になる。

**本リポジトリでは**: `Element::Converted` に `raw: String`(その文字を生んだ
打鍵列)を持たせた。複数文字を出すルール(`kya` → きゃ)は**先頭文字に全打鍵を
載せ、残りは空** — 要素列を走査して raw を連結すれば打鍵列が復元される。

**核心は raw の割り当て**。`evaluate_run` を「run 全体を1回 convert」から
「**打鍵単位のリプレイ**」に変えた:

1. 打鍵を1つずつ prefix に足して `convert(prefix)` し、出力文字数が増えた
   瞬間を「発火」とする
2. 発火が消費した打鍵 = **蓄積打鍵 − 新しい pending の末尾**。converter は
   左から右に処理するので pending は常に蓄積打鍵の末尾に一致する
   (ここを全消しにすると `spam` の `p` のような「まだ pending の打鍵」を
   失う — 実装時に実際に踏んだバグで、テストが捕まえた)
3. 発火バッチ内では、出力中の **ASCII 文字はパススルーの打鍵そのもの**なので
   raw 内の同一文字と整列させ各自1打鍵、非 ASCII の連続(発火したかな)は
   前後の整列点に挟まれた打鍵を先頭文字が持つ。`tっy`(t はパススルーで live の
   まま、っ は `yy` から発火)のような混在もこれで正しく割れる
4. settle(flush)経路も同じリプレイで raw を保持(`ltu` → っ raw="ltu")

**Karukan の不変条件は維持**: 表示・変換・編集(Backspace の「残りを打ち直した
のと同じ結果」)の挙動は一切変えていない。既存371テストが無修正で通ることが
その証明になっている。

**明示した限界**: 発火グループの一部だけ削除した場合(きゃ の ゃ だけ BS)の
raw は best-effort(き に "kya" が残る)。1発火 = 1raw グループという粒度は、
表示1文字 = 1要素の配列では原理的にこれ以上細かくできない(き は "kya" 全体
からしか来ない)。この挙動は専用テストで固定してある。

リプレイは O(run²) の convert 呼び出しになるが、run は「caret で終わる
ローマ字 run」で通常数文字。実測で影響はない。

---

## 12. F9/F10 — 打鍵どおりの英数変換を追加

**Karukan では**: 存在しない操作(F キー未割当)。

**本リポジトリでは**: §11 の raw を使い、F9(全角)/ F10(半角)で打鍵列の
英数変換を追加した。候補は raw の3形(小文字 / 大文字 / 先頭大文字)で、
§10 と同じ「候補リストを持つ通常の Conversion」パターン。

**巡回にも新規状態を持たない**: 現在の候補リストが「この読みの F9(F10)
リスト」とテキスト一致するかで判定し、一致すれば選択 +1(巡回)、もう一方の
幅のリストと一致すれば選択位置を保って幅だけ切替、どちらでもなければ先頭から。

学習は読み(settle 済み。`spam` なら「sぱm」— 単体の s はかなにならず
リテラル)をキーに、かな確定と同じ扱いで記録される。

---

## 13. 範囲指定変換の追加

**Karukan では**: 変換は常にバッファ全体で、読みの一部だけを変換・確定する
手段はなかった。`InputState` に範囲の概念はなく、部分確定という操作も存在
しない。一方で使える部品は眠っていた: `AttributeType::UnderlineDouble`
(選択セグメント用)と `Preedit::from_segments` は**定義済み・未使用**、
Shift+矢印は caret 移動に折り畳まれていて割当が空いていた。

(参考: 移植元の Rakukan はこの機能の実装に失敗している。原因は Windows TSF が
コンポジションを所有し範囲・編集が非同期 edit session 越しになること。Karukan は
フロントエンドが preedit のレンダラに過ぎないため、同じ問題を持たない。)

**本リポジトリでは**: エンジン内の状態設計として実装した
(`karukan-im/core/src/core/engine/range.rs`)。**単純化が設計の本体**:

- 範囲 = **「読みの先頭からの文字数」整数1個**(`range_select: Option<usize>`)。
  anchor は常に先頭固定、任意区間は作らない。新しい状態 enum も作らない
- 解除は `clear_composition` という**既存の全出口1本道**に2フィールドの
  リセットを足しただけ。Escape・Backspace・フォーカス移動で壊れないのは
  この構造による
- 範囲モード中の編集キーは「範囲を落としてから通常処理」— 範囲中の編集は
  意図的に作らない
- ライブ変換表示中に Shift+Right したら先に生読み表示へ戻す(選択対象は常に
  **読み**。変換後表示の上で文字数を数える曖昧さを排除)

**範囲の変換**: `reading[..n]` を既存の `build_conversion_candidates` →
`enter_conversion_state` に渡すだけ。変換キャッシュは(読み, lctx)キーなので
そのまま効く。スコープは `conversion_span: Option<usize>` に記憶し(None =
従来の全体変換で既存経路は不変)、変換 preedit は「選択候補 + 未変換の残り
(下線)」を表示する。読みが範囲を超える予測候補は除外する — 確定すると
選択範囲より多く消費してしまうため。`conversion_span` は `in_composing` /
`cancel_conversion` / `start_conversion` / `clear_composition` で解除され、
範囲変換中の F キー変換・ソース絞り込みはスコープを保ったまま動く。

**部分確定 — 唯一の新設プリミティブ** `commit_range(n)`:

1. 選択候補で `Commit(text)` を発行(学習は §8 のソース方針で記録)
2. `InputBuffer::drain_prefix(n)`(新設)— 先頭 n 要素を除去、caret は相対
   位置を維持、**残りの raw 打鍵は保持**(続けて F10 が効く)。範囲モード
   進入時に settle 済みなので要素と読み文字は1対1
3. 手動 chunk 境界(読み位置)を n だけ左シフト、範囲内の境界は消滅
4. `refresh_input_state()` — 残り読みで composing が続き、ライブ変換が再開

1つの `EngineResult` に Commit → 更新後 preedit の順でアクションが並び、
フロントエンドは既存の適用ループで順に処理する。フォーカスアウト時は
「選択候補 + 未変換の残り読み」を連結して全確定する(部分確定の複雑さを
フォーカス喪失時に持ち込まない)。

**残る検証**: Commit と preedit 更新を1イベントで両立する部分の実アプリ挙動
のみ(端末・ブラウザ・エディタ)。問題が出た場合は2段階確定(確定のみ →
次イベントで preedit 復元)へ逃がす退路を計画に記載してある。

---

## 14. モデルの prefetch と F16 variant

**Karukan では**: `prefetch_all_models` が registry の**全 variant** をダウン
ロードし、macOS の `make install` がこれを呼ぶ。registry は Q5 のみ5 variant
だったのでこれで問題なかったが、大きい variant を追加するとインストール DL が
そのぶん膨らむ構造だった。また非量子化(F16)モデルは registry に無かった。

**本リポジトリでは**:

- 先に prefetch を絞った: `prefetch_variants(ids)` を追加し、
  `karukan-imserver --prefetch-models` は「registry の既定モデル + 設定の
  `model` / `light_model`」だけを温める。`prefetch_all_models` は残るが
  インストーラ経路からは外れる
- その上で `jinen-v2-small-f16`(210MB)/ `jinen-v2-xsmall-f16`(69MB)を
  `models.toml` に登録した。**Q5 既定は不変**で、F16 は config.toml で指定した
  ときだけダウンロードされる
- 登録前に HF リポジトリにファイルが実在することを API で確認した。Karukan の
  モデル解決はキャッシュ優先で「更新はファイル名変更でしか反映されない」設計
  なので、存在しないファイルを登録すると選択時に初めて失敗する — 実在確認は必須
- `model_config.rs` のテストは variant 数をハードアサートしているため 5 → 7 に更新

**計測**(同一コード・基準13ケース・jinen-v2-small): greedy は **13/13 完全
一致**(Q5 の量子化劣化はこのケース集では観測されず)、beam は下位候補のみ
4ケースで相違(優劣拮抗)、greedy 速度は F16 が約1.6倍遅い、DL 81MB vs 210MB。
**Q5 既定の維持が妥当**という結論で、F16 の基準出力は
`docs/baselines/phase0-jinen-v2-small-f16.json` に記録した。

CUDA/Vulkan は未実装 — 現行モデルは CPU で十分速く、動機が生まれた時点で
別ブランチとして扱う。

---

## 15. バージョン表記の追加

**Karukan では**: 動作中のビルドを識別する手段がなかった(cmake 上の 0.1.0 固定)。

**本リポジトリでは**: `karukan-im/core/build.rs` が git 短縮ハッシュ(追跡
ファイルに未コミット変更があれば `-dirty`)をコンパイル時に埋め込み、
`karukan_im::version()` が `0.1.0+<hash>` を返す。表示箇所は fcitx5 の addon
ロード時ログ(FFI `karukan_version()` 経由)と初回モデルロード中の aux、
macOS の `karukan-imserver` 起動ログ。ファイルから直接確認するなら
`strings libkarukan_fcitx5.so | grep -oE '0\.1\.0\+\w+'`。

---

## 付録: 検証の共通手順

各変更で最低限次を実行している。

```bash
cargo fmt --all -- --check
cargo test --workspace --locked   # モデル有り環境
# モデル無し環境の再現:
HF_HOME=$(mktemp -d) HF_ENDPOINT=http://127.0.0.1:1 cargo test --workspace --locked
cargo clippy --workspace          # 警告ゼロを維持
```

変換品質に触る変更では `phase0_baseline` example で基準13ケースの greedy / beam
出力を前後比較し、「意図した改善以外の diff がゼロ」を接続時の条件にした
(手順は [baselines/README.md](baselines/README.md))。

## 付録: 導入した定数一覧

| 定数 | 値 | 場所 | 根拠 |
|---|---|---|---|
| `MAX_GENERATION_BUDGET` | 256 | kanji/backend.rs | 異常入力の時間上限 |
| 生成予算式 | 読み×2+8 | 同上 | byte-fallback 最悪3トークン/文字を2倍+8で覆う |
| `MAX_LEN_RATIO` / `MAX_LEN_SLACK` | 1.5 / 2.0 | kanji/quality.rs | 漢字出力は読みより短いのが通常 |
| `MIN_LEN_RATIO` / `MIN_LEN_READING_CHARS` | 4 / 8 | 同上 | こころざし→志(5:1)を除外しないため8文字未満は不適用 |
| `REPEAT_UNIT_MIN` | 2 | 同上 | 単位1の反復(会社社長)は日常語 |
| `ECHO_MIN_READING_KANA` / `ECHO_HEAD_LEN` | 5 / 5 | 同上 | 短い読みの偶然一致を避ける |
| `DEFAULT_STALE_DAYS` | 0(無効) | learning.rs | 挙動変更は opt-in |
| 辞書監視間隔 | 2秒 | engine/mod.rs | stat 数回/2秒で十分即時 |
