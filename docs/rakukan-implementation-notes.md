# Rakukan 移植の実装解説

このリポジトリで実装した全改修の技術解説。上流 `togatoga/karukan` は AI が作成した
PR を受け付けないため、**人間が各変更の内容・理由・影響を完全に理解し、自分の言葉で
説明・再構成して上流へ提案できる粒度**で書いている。各節が上流への PR 候補1件に
対応し、独立に取り込める順で並べてある。

計画と検証条件は [rakukan-porting-plan.md](rakukan-porting-plan.md)、
比較基準の使い方は [baselines/README.md](baselines/README.md) を参照。

---

## 1. モデル依存テストのスキップ規約統一

**問題**: `llamacpp.rs` の `beam_search_tests` には「モデルが解決できなければ
eprintln して return(スキップ)」という規約が既にあったが、
`backend.rs` のユニットテスト3件・`kanji_conversion_tests.rs` の12件・
`romaji_tests.rs` の1件は `.expect()` で失敗していた。このためオフライン+
HF キャッシュ空の環境では `cargo test --workspace` が通らなかった。

**変更**: 取り残されていた16箇所を既存規約に統一。

```rust
let Some(model) = load_model() else {
    eprintln!("model unavailable, skipping");
    return;
};
```

**ポイント**:
- モデルがキャッシュ済みの環境(CI 含む)では従来どおり全テストが実行される。
  CI の unit ジョブは `--lib --bins` でモデル DL 込みのテストを回しており、
  この挙動は変わらない
- モデル無し環境の再現は「空の `HF_HOME`」+「到達不能な `HF_ENDPOINT`
  (例: `http://127.0.0.1:1`)」で行った(即 connection refused になるので速い)

**変更ファイル**: `karukan-engine/src/kanji/backend.rs`(tests)、
`karukan-engine/tests/kanji_conversion_tests.rs`、`karukan-engine/tests/romaji_tests.rs`

---

## 2. 読み長に応じた生成予算 — `generation_budget`

**問題**: `ConversionConfig::max_new_tokens` は固定50。読みが21文字を超えると
正当な変換でも出力が途中で切れうる。逆に上限を単純に上げると、EOS に到達しない
異常入力で推論時間が読み長と無関係に伸びる。

**設計**: `max(設定値, 読み文字数 × 2 + 8)`、上限 256。

- 係数2の根拠: 漢字出力はおおむね読み1文字 ≦ 1トークン、byte-fallback の
  最悪ケースで1文字3トークン。2倍+スラック8で実在の変換を覆う
- 設定値(既定50)は **floor** として働く: 短い読みでは従来の予算のまま
- 上限256はパスロジカルケースの時間制限

**実装**(`karukan-engine/src/kanji/backend.rs`):

```rust
pub fn generation_budget(reading_chars: usize, configured_max: usize) -> usize {
    (reading_chars.saturating_mul(2).saturating_add(8))
        .max(configured_max)
        .min(MAX_GENERATION_BUDGET) // 256
}
```

`KanaKanjiConverter::convert` の greedy / beam 両経路がこの予算を使う。

**重要な副修正**: greedy 経路(`generate_with_sampler`)は beam と違い、
モデル既定の固定 `n_ctx`(256セル)のコンテキストを使う。予算を上げると
「プロンプト + 生成 > n_ctx」で decode が失敗しうるため、
`llamacpp.rs` の `generate_with_sampler` に防御クランプを入れた:

```rust
let max_new_tokens =
    max_new_tokens.min((self.n_ctx as usize).saturating_sub(input_tokens.len()));
```

beam search は `n_cells = input_len + 2 * beam * (max_new + 1) + headroom` と
自前でセル数を確保するため、この問題を持たない。

**挙動変化**: EOS に到達する通常の変換は不変(基準13ケースの greedy/beam が
完全一致することを確認済み)。読み21文字超で生成上限だけが伸びる。

**テスト**: `generation_budget` の floor / スケール / 上限の単体テスト3件
(モデル非依存)。

---

## 3. EOS 未到達候補の区別 — `BeamCandidate`

**問題**: beam search の `finalize_beams` は、EOS に到達した beam
(`finished_beams`)と生成上限で打ち切られた beam(`beams`)を**1つの
リストに連結してスコア順に返していた**。戻り値は `Vec<(Vec<LlamaToken>, f32)>`
で完了フラグを持たず、呼び出し側は途中で切れた文章を通常候補と区別できない。

**設計**: 戻り値型を変更して事実を運ぶ。ポリシー(切り捨てるかどうか)は
llamacpp 層ではなく backend 層に置く。

```rust
pub struct BeamCandidate {
    pub tokens: Vec<LlamaToken>,
    pub score: f32,     // 累積 log-prob(従来から内部にあったが API 境界で捨てられていた)
    pub finished: bool, // EOS 到達か、生成上限による打ち切りか
}
```

- `generate_beam_search` / `generate_beam_search_full_eval`(等価性テスト用の
  参照実装)の両方を `Vec<BeamCandidate>` に変更
- fast/reference の等価性テストは **finished フラグも含めて**比較するよう強化
- `score` を運ぶようにしたのは confidence フィルタ(§6)への布石 —
  型変更を2回やらないため

**ポリシー**(`backend.rs` の `convert` beam 経路):

```rust
let (complete, truncated): (Vec<_>, Vec<_>) = results.into_iter().partition(|c| c.finished);
// truncated は debug ログに数を出して捨てる。complete だけが候補になる
```

全 beam が打ち切りだった場合は、既存の「候補ゼロなら読みを push する」
フォールバックがそのまま効く(この安全網は元からある。壊さないことが条件)。

**挙動変化**: §2 の予算スケールにより通常の変換で打ち切りはほぼ発生しないため、
実用上の出力は不変(基準13ケース一致)。異常入力での途中切れ候補だけが消える。

**テスト**: 「3トークンの予算では全 beam が unfinished、50トークンなら EOS 到達」
を検証するモデルテスト(`budget_cut_beams_are_flagged_unfinished`)。

**上流 PR 時の注意**: 戻り値型の変更なので、`karukan-cli/src/bin/server.rs` の
2箇所(タプルへの `.map(|c| (c.tokens, c.score))` で吸収)とテスト・ベンチの
機械的更新が同じ PR に入る。

---

## 4. 退化候補フィルタ — `degenerate_reason`

**問題**: モデル出力の検査は trim・空文字除去・完全一致 dedup のみ。小さい
モデルは「変換になっていない出力」— 入力かなのエコー、暴走反復、極端な長短、
ルビ状出力 — を出すことがあり、それがそのまま候補に並んでいた。

**設計**: モデル非依存の純粋関数として分類器を新設(`karukan-engine/src/kanji/quality.rs`)。
閾値は保守側に倒す — 「悪い候補を時々通す」フィルタは鬱陶しいだけだが、
「正しい変換を食う」フィルタはバグである。

```rust
pub enum DegenerateReason { Empty, TooShort, TooLong, Repetition, PrefixEcho, RubyEcho }
pub fn degenerate_reason(candidate: &str, reading: &str) -> Option<DegenerateReason>
```

**判定順序に意味がある**: エコー系(RubyEcho / PrefixEcho)を長さ系より先に
判定する。ルビ状出力は読みより長く、かなエコーは短いので、順序を逆にすると
理由が TooLong / TooShort に化けてログの診断価値が落ちる。

各判定の詳細:

| 理由 | 条件 | 誤検出回避 |
|---|---|---|
| `Empty` | 空・空白のみ | — |
| `RubyEcho` | 括弧(半角/全角)の中身が**かなだけ**(明日(あした)) | 「株式会社(仮)」のようにかな以外を含む括弧書きは通す |
| `PrefixEcho` | かなのみ、かつ読みの**真の前方一致**(カタカナはひらがな正規化して比較) | 読み**全体**と等しいかな候補は正当(かな語がそれ自身に変換されるケース)なので通す |
| `TooLong` | 候補 > 読み × 1.5 + 2 文字 | 漢字出力は通常読みより**短い**ので、1.5倍+2 は暴走にしか当たらない |
| `TooShort` | 候補 × 4 < 読み(読み8文字以上のときのみ) | 短い読みは正当に強く圧縮する(こころざし→志は5:1)ため、8文字未満には適用しない |
| `Repetition` | 長さ2以上の単位の隣接反復が候補にあり、**読みには無い** | 読み自身が反復するなら検査ごと停止(わかったわかった、とうきょうとっきょきょかきょく — 後者は「きょきょ」が隣接反復)。単位1の反復(会社社長)は日常の日本語なので対象外 |

**接続**(`backend.rs::convert`): greedy / beam 両経路で候補追加前に検査し、
除外は理由付き debug ログ。全滅時は読みフォールバック。**LRU キャッシュより
内側**(格納前)なので、退化候補は再推論されず消えたままになる — 閾値を
設定化する場合はこの位置を再検討する、と計画に明記してある。

**テスト**: モデル非依存の単体テスト11件。「正当なものを通す」側のテスト
(東京特許許可局・承る・すもももももももものうち・会社社長・株式会社(仮))が
「悪いものを落とす」側と同数以上あるのが要点。

---

## 5. context エコー除外 — `echo_free_context`

**問題**: 変換コンテキスト(lctx)に**未変換のかな文**が混じると、モデルは
変換せずかなをエコーしやすい。しかもエコーを能動的に作る経路が実在する:
chunk のモデル変換が空振りすると `convert_chunk` が生かな読みへフォールバック
し、それが `preceding_converted` に連結されて**次 chunk の lctx にそのまま入る**。
Phase 0 の基準ケース echo-02(context「あしたはあめかもしれないとおもった」+
読み「あしたはあめ」)で、greedy が生かな「あしたはあめ」を返すことを実測記録した。

**設計**(`quality.rs::echo_free_context`):

- context を文単位(。．！？!? と改行)に分割し、**読みの先頭かな5文字を
  含む漢字なしの文だけ**を除去、他の文は原文のまま残す
- 読みのかなが5文字未満なら何もしない(短い読みは偶然一致しやすい)
- **漢字を含む文は絶対に除去しない** — それは変換済みテキストであり、かなが
  読みと重なっていてもモデルに残すべきコンテキストそのもの
- カタカナはひらがな正規化して比較(カタカナのエコーも捕まえる)

**適用位置が設計の要**: エンジンの `convert` 内、つまり **IM 側の変換キャッシュ
の内側**でプロンプト構築直前に適用する。これにより:

1. IM のキャッシュキー(生の lctx)は不変 — 除外の発動有無でキーが揺れて
   キャッシュヒット率が落ちる副作用がない
2. 呼び出し側が保持する context(原文)は書き換わらない
3. すべての呼び出し元(IM の chunk 変換、CLI サーバー)が同じ対策を受ける

発動は debug ログで観測可能。

**挙動変化(実測)**: echo-02 の greedy が「あしたはあめ」→「明日は雨」に改善。
echo-01 の beam からかなエコー候補が消滅。対照例 echo-03(漢字混じり context)
と他の10ケースは完全一致。

**テスト**: 単体6件(同一文の除去・該当文のみ除去で前後保持・漢字文の保持・
短い読みの不適用・カタカナエコー・無関係かな文の保持)。

---

## 6. confidence 観測ログ(段階導入の第1段)

beam 候補の平均 log-prob(`score / 生成トークン数`)を debug ログに出すだけで、
除外は行わない。実データ収集後に `confidence_margin`(最良候補との差)による
除外を別途検討する。§3 で `score` が API 境界を越えるようになったため、
この段は数行で済む。**最良候補自体を棄却する絶対閾値は既定で無効にする方針**
(計画 §1-E)。

---

## 7. 学習の期限切れ整理 — `stale_days`

**問題**: 学習キャッシュ(TSV)は読み込み時に一切掃除されない。エントリが
消えるのは手動削除(Ctrl+Backspace)か、保存時の `max_entries`(既定10,000)
超過だけ。TSV には `last_access`(Unix 秒)列が**元からある**のに使われていない。

**実装**(`karukan-engine/src/learning.rs`):

- `LearningConfig` に `stale_days: u32` を追加(**0 = 無期限、これが既定**)
- `load` の末尾で `remove_stale(now)` を呼び、`last_access` が
  `now - stale_days × 86400` より古いエントリを削除
- **境界は含める**(経過 == stale_days は保持。「stale_days 日を超えたら」期限切れ)
- 削除があった場合だけキャッシュを dirty にし、次回保存で TSV へ反映

既定を 0(無効)にしたのは計画の基本方針「挙動変更には設定または段階導入を
用意する」(§2-5)による。スコア式(`10/(1+経過日数) + ln(1+頻度)`)が古い
エントリを既に強く沈めているため、削除は掃除であって品質改善の主役ではない。

**設定の配管**: `[learning] stale_days` を `LearningSettings` と
`default.toml`(コメント付き)に追加し、`init_learning_cache` が渡す。

**テスト**: 無効時は何も消えない / 削除+dirty / 期限内保持 / 境界 /
保存後に再読込しても復活しない、の5件 + 設定テスト。

---

## 8. ソース別学習方針 — `records_learning`

**問題**: `record_learning(reading, surface)` の条件は Emoji モード除外と
`max_surface_chars`(50文字)だけで、**何を確定しても学習される**。特に:

1. モデルが変換を出せなかったときの**読みフォールバック候補**を確定すると、
   「変換が無かったこと」が学習される(誤学習の主因)
2. 候補ウィンドウを開かない**ライブ変換の確定**で、モデルの第一候補が
   誤っていてもそのまま学習され、次回から最上位に来る(自己強化)

**設計**(`karukan-im/core/src/core/candidate.rs`):

```rust
impl CandidateSource {
    pub fn records_learning(&self, explicit: bool) -> bool {
        match self {
            Fallback => false,                    // 常に学習しない
            Model | Rewriter => explicit,         // 明示選択時のみ
            Learning | UserDictionary | Dictionary => true, // 常に学習
        }
    }
}
```

`explicit` = 候補ウィンドウで自分が選んだ確定(true)か、表示中のものを
そのまま受け入れた確定(ライブ変換の Enter、false)か。

**決定事項(2026-08-24)**: かな確定(`surface == reading`、ソースなし)は
mozc と同様**学習に含める** — よく使うかな語を上位に押し上げる働きを残す。
`surface != reading` の除外条件は導入しない。モデル空振り時の生かなは
Fallback **ソース**として除外されるので、この方針でも汚染は起きない。

**配線**: 確定経路は3つあり、すべてにソース/学習可否を通した。

| 経路 | 実装 | explicit |
|---|---|---|
| 変換確定(Enter) | `selected_conversion_info` がソースも返し、`finish_conversion` が判定 | true(ウィンドウが開いていた) |
| Composing 確定(ライブ変換 Enter) | `resolve_composing_commit` の戻り値に学習可否を追加。ライブテキスト = Model の implicit | false |
| フォーカスアウト(`commit()`) | 上記2経路と同じ判定を共有 | 状態に応じて同上 |

**挙動変化**: (1) 読みフォールバック確定が学習されない、
(2) 候補ウィンドウなしのライブ変換確定が学習されない。
候補を自分で選んだ確定・かな/カタカナ確定は従来どおり。

**テスト**: ポリシー全組み合わせの単体テスト + 統合4件(Fallback 確定・
ライブ確定・かな確定・学習候補の明示選択)。

---

## 9. ユーザー辞書の hot reload — `UserDictWatcher`

**問題**: `user_dicts/` は IME 初期化時に一回だけスキャンされ
(`init_user_dictionaries` は2回目以降早期 return)、編集の反映に IME 再起動が
必要だった。

**設計**(`karukan-im/core/src/core/engine/user_dicts.rs`、新規):

- **fingerprint 監視**: ファイルごとに(パス, mtime, サイズ)の stamp を取り、
  変化したファイルだけ再パースする。1回のチェックはファイル数回の stat のみ
- **ファイル単位の last-good キャッシュ**: `BTreeMap<PathBuf, UserDictFile>` に
  「最後の読込試行時の stamp」と「最後に**成功**した読込のパース済み辞書」を持つ。
  壊れた編集(KRKN マジック + 不正データ等)は直前の正常な内容を使い続け、
  stamp は進める — **ファイルが次に変わるまで再試行しない**ので、警告ログの
  連打も起きない
- `BTreeMap` はパス順(アルファベット順)を保ち、従来の一回読みのマージ
  優先順位をそのまま維持する
- 存在しないディレクトリは空として扱う — 後からディレクトリを作っても
  通常の変更として検出される
- `refresh()` の戻り値: `None` = 変化なし / `Some(merged)` = 変化あり
  (`merged` は全ファイルが消えたときに `None`)

**前提リファクタ**: `Dictionary::merge` は `Vec<Dictionary>` を消費する形
だったが、`Dictionary` は `Clone` を持たないためファイル単位キャッシュと
両立しない。`IntoIterator<Item = &Dictionary>` を受けてエントリを clone する
形に変更した(挙動不変、呼び出し3箇所を機械的更新)。

**ポーリング**: `process_key` 冒頭(`poll_loaded_models` の隣)から
**2秒スロットル**で `refresh()` を呼ぶ。打鍵毎のコストはタイムスタンプ比較
1回、2秒に1回だけ stat 数回。init は watcher の初回 refresh に一本化した。

**同期性の注意**: 再読込は key イベントスレッド上で同期実行される。ユーザー
辞書は小さい前提で、巨大辞書はバイナリ(KRKN)化が既存ドキュメントで案内
されている。

**テスト**: watcher 単体7件(初回マージ・無変更・編集・削除・最終ファイル
削除・壊れた編集の last-good と再試行抑制・ディレクトリ後作成)+
エンジン統合4件(poll 経由の追加/編集/削除、スロットルが効くこと)。
テストでは mtime の秒精度衝突を避けるため、編集はバイト長が変わる内容で行う。

---

## 10. F6/F7/F8 — かな系固定変換

**設計の要**: 新しい状態を作らず、**「変換候補3つ(ひらがな・全角カタカナ・
半角カタカナ)を持つ通常の Conversion 状態」**として実装する
(`karukan-im/core/src/core/engine/fkeys.rs`)。

- 押した F キーの変換が選択された状態で `enter_conversion_state` に入る。
  F キー同士は**同じリスト内の選択移動**、Enter は通常の確定経路(かな確定
  として学習)、Escape は編集可能な composing に戻る — すべて既存機構
- 読みは Space 変換と同じく `settled_reading`(ローマ字 tail を settle した
  読み。「あいk」→「アイk」)。composing / conversion のどちらからでも入れる
- 変換先はすべて表示かなから導出できる: `hiragana_to_katakana` /
  `katakana_to_half_width`(いずれも `kana.rs` の既存関数)
- かなを含まない読みでは3変換が同一になるため、text で dedup して選択位置を
  合わせる
- 未入力時はアプリへ透過(not_consumed)、絵文字モードでは no-op

**キー伝達は両フロントエンドで最初から到達済み**だった点が重要:
fcitx5 addon は全キーを verbatim で core へ転送し、macOS の `KeyCodeMap.swift`
は F1–F12 → keysym 0xffbe–0xffc9 のマップを持っている。必要だったのは
core の `keycode.rs` に F6–F10 定数を足すことと、composing / conversion の
キーディスパッチへの2アームだけ。**フロントエンドの変更はゼロ**。

**テスト**: 統合9件(各変換・相互切替・確定と学習・Esc 復帰・tail settle・
変換状態からの切替・Empty 透過・絵文字 no-op)。

---

## 11. 生打鍵履歴(raw input)— F9/F10 の前提

**問題**: F9/F10 は mozc 的には**打鍵したキーそのまま**への変換
(「わたし」を `watasi` と打ったなら `watasi`。かなの再ローマ字化では
`watashi` になってしまい別物)。しかし Karukan の `InputBuffer` は発火した
打鍵を表示文字 `Converted(かな)` に**再記録して元の ASCII 列を捨てる**設計で、
復元できなかった。

**設計**(`karukan-im/core/src/core/engine/input_buffer.rs`):

`Element::Converted` に `raw: String`(その文字を生んだ打鍵列)を追加。
複数文字を出すルール(`kya` → きゃ)は**先頭文字に全打鍵を載せ、残りは空**。
要素列を走査して raw を連結すれば打鍵列が復元される。

**核心は raw の割り当てアルゴリズム**。`evaluate_run` を「run 全体を1回
convert」から「**打鍵単位のリプレイ**」に変更した:

1. 打鍵を1つずつ prefix に足して `convert(prefix)` し、出力文字数が増えた
   瞬間を「発火」とする
2. 発火が消費した打鍵 = **蓄積打鍵 − 新しい pending の末尾**。
   (converter は左から右に処理するので、pending は常に蓄積打鍵の末尾に
   一致する。ここを `clear()` で全消しすると `spam` の `p` のように
   「まだ pending の打鍵」を失う — 実装時に踏んだバグ)
3. 発火バッチ内の割り当て: 出力中の **ASCII 文字はパススルーの打鍵そのもの**
   なので raw 内の同一文字と整列させ各自1打鍵(`y1ka` → y,1 は自分自身)。
   非 ASCII の連続(発火したかな)は、前後の整列点に挟まれた打鍵を先頭文字が
   持つ。これで `tっy`(t はパススルーで live のまま、っ は `yy` から発火)の
   ような混在も正しく割れる
4. settle(flush)経路も同じリプレイ + 残り pending の `flush_pending` で
   raw を保持(`ltu` → っ raw="ltu")

**維持した不変条件**: 表示・変換・編集の挙動は一切変えない。既存371テスト
(バックスペースの等価性テスト含む)が無修正で通ることがその証明。

**明示した限界**: 発火グループの一部だけを削除した場合(きゃ の ゃ だけ BS)、
raw は best-effort(き に "kya" が残る)。この挙動は専用テストで**固定**して
あり、変えるなら意図的な変更になる。1発火 = 1raw グループという粒度は、
per-display-char 要素配列では原理的にこれ以上細かくできない(き は "ky" からも
"ki" からも来ない — "kya" 全体からしか来ない)。

**性能**: リプレイは O(run²) の convert 呼び出しだが、run は「caret で終わる
ローマ字 run」で通常数文字。実測影響なし。

---

## 12. F9/F10 — 英数変換

`fkeys.rs` に追加。§10 と同じ「候補リストを持つ通常の Conversion」パターン:

- 候補は raw 打鍵の3形: 小文字 / 大文字 / 先頭大文字(F9 は全角化、F10 は半角)
- **巡回の実装に新規状態を持たない**: 現在の候補リストが「この読みの F9(F10)
  リスト」と一致するかをテキスト比較で判定し、一致すれば選択を +1(巡回)、
  **もう一方の幅のリスト**と一致すれば同じ選択位置のまま幅だけ切替、
  どちらでもなければ先頭から
- 読み(学習キー・Esc 復帰用)は settle 済みの読み。確定はかな確定として学習
  される(例: `spam` の読みは「sぱm」— 単体の s はかなにならずリテラル)

**テスト**: 統合6件(打鍵復元・全角・巡回とラップ・幅切替で大小保持・
記号/パススルー込みの復元・確定と学習キー)。

---

## 13. 範囲指定変換

**Rakukan で失敗した機能**。失敗要因は TSF がコンポジションを所有し範囲・編集が
非同期 edit session 越しになること。Karukan はフロントエンドが preedit の
レンダラに過ぎないため、**純粋なエンジン内状態設計の問題**に還元される。
設計の全文は [rakukan-porting-plan.md](rakukan-porting-plan.md) の Phase 4 節。

**単純化が設計の本体**(`karukan-im/core/src/core/engine/range.rs`):

- 範囲 = **「読みの先頭からの文字数」整数1個**(`range_select: Option<usize>`)。
  anchor は常に先頭固定。新しい状態 enum は作らない
- 解除は `clear_composition` という**既存の全出口1本道**に足した2行
  (`range_select = None; conversion_span = None`)。Escape・Backspace・
  フォーカス移動で壊れないのはこの構造による
- 範囲モード中の編集キーは「範囲を落としてから通常処理」— 範囲中の編集は
  意図的に作らない
- ライブ変換表示中に Shift+Right したら、先に生読み表示へ戻してから範囲モード
  に入る(選択対象は常に**読み**。変換後表示の上で文字数を数える曖昧さを排除)

**表示**: 定義済み・未使用だった `AttributeType::UnderlineDouble`(選択
セグメント用)と `Preedit::from_segments` をそのまま使用。

**範囲の変換**: `reading[..n]` を既存の `build_conversion_candidates` →
`enter_conversion_state` に渡すだけ。変換キャッシュは(読み, lctx)キーなので
そのまま効く。スコープは `conversion_span: Option<usize>` に記憶し
(None = 従来の全体変換)、変換 preedit は「選択候補 + 未変換の残り(下線)」
を表示する(構築を `conversion_preedit` に一元化し、候補移動・ソース絞り込み
でも残りが見え続ける)。**読みが範囲を超える予測候補は除外**する — 確定すると
選択範囲より多く消費してしまうため。

`conversion_span` の解除点: `in_composing`(変換が composing に解ける全経路)、
`cancel_conversion`、`start_conversion`(全体変換の開始)、`clear_composition`。
範囲変換中の F キー変換・ソース絞り込みはスコープを保ったまま動く(読みが
prefix のままなので、それが正しい)。

**部分確定 — 唯一の新設プリミティブ** `commit_range(n)`:

1. 選択候補で `Commit(text)` を発行(学習はソース方針で記録)
2. `InputBuffer::drain_prefix(n)` — 先頭 n 要素を除去、caret は相対位置を
   維持、**残りの raw 打鍵は保持**(続けて F10 が効く)。範囲モード進入時に
   settle 済みなので要素と読み文字は1対1
3. 手動 chunk 境界(`chunk_breaks`、読み位置)を n だけ左シフト、範囲内の
   境界は消滅
4. `refresh_input_state()` — 残り読みで composing が続き、ライブ変換が再開

1つの `EngineResult` に **Commit → 更新後 preedit** の順でアクションが並ぶ。
フロントエンドは既存の適用ループで順に処理するだけ。

**フォーカスアウト**(`commit()`)中の範囲変換は「選択候補 + 未変換の残り読み」
を連結して全確定する(部分確定の複雑さをフォーカス喪失時に持ち込まない)。

**残る検証**: 「Commit と preedit 更新の1イベント両立」の実アプリ挙動のみ
(端末・ブラウザ・エディタで要確認)。問題が出た場合の退路は2段階確定
(このイベントで確定のみ → 次イベントで preedit 復元)。

**テスト**: 統合11件(選択・クランプ・縮小と解除・Esc・編集キーでの解除・
prefix 変換・予測除外・部分確定とライブ再開/raw 保持・全範囲確定・
変換からの Esc 復帰・フォーカスアウト)。

---

## 14. prefetch 絞り込みと F16 variant

**順序が重要**: F16 を registry に足す**前に** prefetch を絞る。
`prefetch_all_models` は registry の全 variant をダウンロードし、macOS の
`make install` が呼ぶため、先に絞らないとインストール DL が 279MB 増える。

- `prefetch_variants(ids)` を追加し、`karukan-imserver --prefetch-models` は
  「registry の既定モデル + 設定の `model` / `light_model`」だけを温める。
  `prefetch_all_models` は残るがインストーラ経路からは外れる
- `jinen-v2-small-f16`(210MB)/ `jinen-v2-xsmall-f16`(69MB)を
  `models.toml` に登録(HF リポジトリにファイルが実在することを API で確認
  済み — **モデル更新はファイル名変更でしか反映されない**キャッシュ設計なので、
  存在しないファイルの登録は選択時に初めて失敗する。登録前の実在確認は必須)
- `model_config.rs` のテストは variant 数をハードアサートしているため 5 → 7 に更新

**計測**(同一コード・Phase 0 基準13ケース・jinen-v2-small):
greedy **13/13 完全一致**(量子化劣化はこのケース集では観測されず)、
beam は下位候補のみ4ケースで相違(優劣拮抗)、greedy 速度は F16 が約1.6倍遅い、
DL 81MB vs 210MB。→ **Q5 既定の維持が妥当**。F16 の基準出力は
`docs/baselines/phase0-jinen-v2-small-f16.json` に記録
(生成コード時点の注意は baselines/README.md 参照)。

CUDA/Vulkan(5-A)は未実装 — 現行モデルは CPU で十分速く、動機が生まれた
時点で別ブランチとして着手する方針。

---

## 15. バージョン表記

どのビルドが動いているかを実機で確認できるようにする。

- `karukan-im/core/build.rs` が git 短縮ハッシュ(追跡ファイルに未コミット
  変更があれば `-dirty`)をコンパイル時に埋め込み、`karukan_im::version()` が
  `0.1.0+<hash>` を返す(git 外ビルドは `+unknown`)
- fcitx5: FFI `karukan_version()` を追加し、addon 構築時に FCITX_INFO ログへ
  「Karukan <version>」、初回モデルロード中の aux 表示にも付記
- macOS: `karukan-imserver` 起動時の info ログ
- ファイルから直接確認: `strings libkarukan_fcitx5.so | grep -oE '0\.1\.0\+\w+'`

---

## 付録: 検証の共通手順

各変更で最低限次を実行した。

```bash
cargo fmt --all -- --check
cargo test --workspace --locked   # モデル有り環境
# モデル無し環境の再現:
HF_HOME=$(mktemp -d) HF_ENDPOINT=http://127.0.0.1:1 cargo test --workspace --locked
cargo clippy --workspace          # 警告ゼロを維持
```

変換品質に触る変更では、`phase0_baseline` example で基準13ケースの
greedy / beam 出力を前後比較した(手順は [baselines/README.md](baselines/README.md))。
「意図した改善以外の diff がゼロ」を各接続コミットの条件にしている。

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
