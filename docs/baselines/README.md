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

注意: `phase0-jinen-v2-small-q5.json` は **Phase 1 導入前**のコードで生成した
基準(だからこそ 1-D の改善が diff で見える)。一方
`phase0-jinen-v2-small-f16.json` は **Phase 1 導入後**のコードで生成した
F16 の記録なので、Q5 と F16 を比較するときは同じコードで Q5 を再実行して
比べること(Phase 5-B の計測では greedy 13/13 一致、beam は下位候補のみ
4ケースで相違、greedy 速度は F16 が約1.6倍遅い、DL 81MB vs 210MB だった)。
