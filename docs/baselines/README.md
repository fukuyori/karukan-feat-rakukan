# Phase 0 基準出力

[Rakukan 機能の選択移植計画](../rakukan-porting-plan.md) の Phase 0 で固定した、
変換品質変更の比較基準。

- `phase0-cases.json` — 異常入力ケース集。カテゴリは計画 Phase 0 の分類に対応する
  (`long_reading` / `repeat_prone` / `legit_repeat` / `context_echo` /
  `context_echo_control` / `digits`)。
- `phase0-<model>.json` — そのモデルでの greedy / beam 変換の基準出力と所要時間。

再生成:

```bash
cargo run -p karukan-engine --release --example phase0_baseline -- \
    docs/baselines/phase0-cases.json > docs/baselines/phase0-jinen-v2-small-q5.json
```

`--model VARIANT_ID` でモデルを、`--beam N` で beam 幅(既定 3)を、
`--dict PATH` で辞書検索の併記を指定できる。

出力は判定を持たない記録であり、Phase 1 のフィルタ導入前後で同じケース・
同じモデル・同じスレッド数の出力を diff で比較するために使う。
所要時間(`*_ms`)は環境依存なので、性能比較は同一マシンでの前後比較に限る。
