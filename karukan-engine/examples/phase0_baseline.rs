//! Phase 0 基準出力ダンプ (docs/rakukan-porting-plan.md)
//!
//! 異常入力ケース集を greedy / beam でモデル変換し、任意で辞書検索も添えて
//! JSON で標準出力へ書き出す。Phase 1 の品質変更前後で同じケース・同じモデルの
//! 出力を比較するための記録用で、判定は行わない。
//!
//! 使い方:
//!
//! ```text
//! cargo run -p karukan-engine --release --example phase0_baseline -- \
//!     docs/baselines/phase0-cases.json \
//!     [--model VARIANT_ID] [--beam N] [--dict PATH]
//! ```
//!
//! モデル既定は registry の default_model、beam 幅既定は 3
//! (karukan-im の `beam_width` 既定と同じ)。

use std::time::Instant;

use karukan_engine::kanji::registry;
use karukan_engine::{Backend, Dictionary, KanaKanjiConverter};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Case {
    id: String,
    category: String,
    reading: String,
    #[serde(default)]
    context: String,
    #[serde(default)]
    note: String,
}

#[derive(Serialize)]
struct DictBaseline {
    exact: Vec<String>,
    predictive: Vec<String>,
}

#[derive(Serialize)]
struct CaseBaseline {
    id: String,
    category: String,
    reading: String,
    context: String,
    note: String,
    greedy: Vec<String>,
    greedy_ms: u128,
    beam: Vec<String>,
    beam_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    dict: Option<DictBaseline>,
}

#[derive(Serialize)]
struct Baseline {
    model: String,
    beam_width: usize,
    cases: Vec<CaseBaseline>,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut cases_path = None;
    let mut model_id = None;
    let mut beam_width = 3usize;
    let mut dict_path = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => model_id = args.next(),
            "--beam" => {
                beam_width = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .expect("--beam requires a number")
            }
            "--dict" => dict_path = args.next(),
            _ => cases_path = Some(arg),
        }
    }
    let cases_path = cases_path
        .expect("usage: phase0_baseline CASES.json [--model ID] [--beam N] [--dict PATH]");

    let model_id = model_id.unwrap_or_else(|| registry().default_model.clone());
    let json = std::fs::read_to_string(&cases_path).expect("failed to read cases file");
    let cases: Vec<Case> = serde_json::from_str(&json).expect("invalid cases JSON");

    let backend = Backend::from_variant_id(&model_id).expect("failed to resolve model");
    let converter = KanaKanjiConverter::new(backend).expect("failed to load model");
    let dict = dict_path.map(|p| Dictionary::load_auto(&p).expect("failed to load dictionary"));

    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        eprintln!("converting {} ...", case.id);

        let start = Instant::now();
        let greedy = converter
            .convert(&case.reading, &case.context, 1)
            .unwrap_or_else(|e| vec![format!("<error: {e}>")]);
        let greedy_ms = start.elapsed().as_millis();

        let start = Instant::now();
        let beam = converter
            .convert(&case.reading, &case.context, beam_width)
            .unwrap_or_else(|e| vec![format!("<error: {e}>")]);
        let beam_ms = start.elapsed().as_millis();

        let dict_baseline = dict.as_ref().map(|d| DictBaseline {
            exact: d
                .exact_match_search(&case.reading)
                .map(|r| r.candidates.iter().map(|c| c.surface.clone()).collect())
                .unwrap_or_default(),
            predictive: d
                .predictive_search(&case.reading, 10)
                .iter()
                .map(|m| m.candidate.surface.clone())
                .collect(),
        });

        results.push(CaseBaseline {
            id: case.id,
            category: case.category,
            reading: case.reading,
            context: case.context,
            note: case.note,
            greedy,
            greedy_ms,
            beam,
            beam_ms,
            dict: dict_baseline,
        });
    }

    let baseline = Baseline {
        model: model_id,
        beam_width,
        cases: results,
    };
    println!("{}", serde_json::to_string_pretty(&baseline).unwrap());
}
