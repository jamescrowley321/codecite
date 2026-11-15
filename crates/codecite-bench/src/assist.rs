//! What `codecite assist` costs the agent's grep call it rides on: its
//! wall time and how often it has anything to add.

use std::path::Path;

use anyhow::Context as _;
use serde::Serialize;

use crate::Query;
use crate::cost::{fastest, percentile, run_timed};

/// The latency of a set of assist calls.
#[derive(Debug, Serialize)]
pub struct AssistCost {
    pub calls: usize,
    pub median_ms: f64,
    pub p95_ms: f64,
    /// The share of calls that printed a block.
    pub non_empty: f64,
    /// The median size of a printed block; empty calls are left out.
    pub median_block_bytes: u64,
}

impl AssistCost {
    /// The cost of a set of `(bytes, milliseconds)` measurements, one per
    /// call.
    pub fn of(measured: &[(u64, f64)]) -> AssistCost {
        let mut ms: Vec<f64> = measured.iter().map(|m| m.1).collect();
        ms.sort_by(f64::total_cmp);
        let mut blocks: Vec<f64> = measured
            .iter()
            .filter(|m| m.0 > 0)
            .map(|m| m.0 as f64)
            .collect();
        blocks.sort_by(f64::total_cmp);
        AssistCost {
            calls: measured.len(),
            median_ms: percentile(&ms, 0.5),
            p95_ms: percentile(&ms, 0.95),
            non_empty: blocks.len() as f64 / measured.len().max(1) as f64,
            median_block_bytes: percentile(&blocks, 0.5).round() as u64,
        }
    }
}

/// The grep inputs a query stands for: its text as a free-text pattern,
/// and its first identifier-like word as a symbol pattern, the two shapes
/// of call assist treats differently.
pub fn grep_inputs(text: &str) -> Vec<String> {
    let mut inputs = vec![serde_json::json!({ "pattern": text }).to_string()];
    let symbol = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .find(|w| w.starts_with(|c: char| c.is_alphabetic() || c == '_'));
    if let Some(symbol) = symbol {
        inputs.push(serde_json::json!({ "pattern": symbol }).to_string());
    }
    inputs
}

/// Runs `codecite assist <corpus> --tool grep` on every query's
/// [`grep_inputs`], with no tool output on stdin, and reports the free-text
/// calls, the symbol calls and both together.
pub fn assist(
    queries: &[&Query],
    corpus: &str,
    codecite: &Path,
) -> anyhow::Result<serde_json::Value> {
    let (mut text, mut symbol) = (Vec::new(), Vec::new());
    for query in queries {
        for (i, input) in grep_inputs(&query.text).into_iter().enumerate() {
            let args = ["assist", corpus, "--tool", "grep", "--input", &input].map(str::to_owned);
            let measured = fastest(|| run_timed(codecite, &args))
                .with_context(|| format!("query `{}`", query.id))?;
            if i == 0 { &mut text } else { &mut symbol }.push(measured);
        }
    }
    let all: Vec<(u64, f64)> = text.iter().chain(&symbol).copied().collect();
    Ok(serde_json::json!({
        "engine": "assist",
        "free_text": AssistCost::of(&text),
        "symbol": AssistCost::of(&symbol),
        "all": AssistCost::of(&all),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_is_called_as_free_text_and_as_its_first_identifier() {
        assert_eq!(
            grep_inputs("validate_logout_token sid check"),
            [
                r#"{"pattern":"validate_logout_token sid check"}"#,
                r#"{"pattern":"validate_logout_token"}"#
            ]
        );
        // Leading punctuation and numbers are not an identifier.
        assert_eq!(grep_inputs("\"42 jti\" replay")[1], r#"{"pattern":"jti"}"#);
        assert_eq!(grep_inputs("4.3 --").len(), 1);
    }

    #[test]
    fn p95_and_the_share_of_calls_that_printed_a_block() {
        let measured: Vec<(u64, f64)> = (1..=20)
            .map(|i| (if i % 4 == 0 { 0 } else { 100 * i }, i as f64))
            .collect();
        let c = AssistCost::of(&measured);
        assert_eq!(c.calls, 20);
        assert!((c.p95_ms - 19.05).abs() < 1e-9, "{}", c.p95_ms);
        assert!((c.median_ms - 10.5).abs() < 1e-9);
        assert!((c.non_empty - 0.75).abs() < 1e-9);
        // Blocks of 100..1900 without every fourth: the median of 15.
        assert_eq!(c.median_block_bytes, 1000);
    }

    #[test]
    fn no_calls_cost_nothing() {
        let c = AssistCost::of(&[]);
        assert_eq!((c.calls, c.median_block_bytes), (0, 0));
        assert_eq!(c.non_empty, 0.0);
    }
}
