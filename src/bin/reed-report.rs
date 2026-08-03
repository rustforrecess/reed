//! reed-report — judge evidence-record files and render the tiered report.
//!
//! The CLI end of the adapter seam: ANY producer that writes evidence-core
//! records is reportable — a11y-agent's `evidence.json` today, a heddle
//! host's stored passes tomorrow — with the judging policy supplied as
//! data, not code.
//!
//!   reed-report evidence.json [more.json ...]
//!       [--rules FILE]     Datalog policy (default: none — admit all)
//!       [--admit PRED]     admission predicate (default: reportable,
//!                          only when --rules is given)
//!       [--semiring boolean|maxmin|probability]   (default: maxmin)
//!       [--title TEXT]     report heading
//!       [--json FILE]      also write the verdicts as JSON
//!
//! The Markdown report goes to stdout. Example — the verified-only tier of
//! an a11y audit:
//!
//!   echo "reportable(F) :- verified(F)." > verified.dl
//!   reed-report reports/run-*/evidence.json --rules verified.dl > audit.md

use std::fs;
use std::process::ExitCode;

use evidence_core::Evidence;
use reed::{Config, Semantics, SemiringChoice, judge, report};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut files: Vec<String> = Vec::new();
    let mut rules_file: Option<String> = None;
    let mut admit: Option<String> = None;
    let mut semiring = SemiringChoice::MaxMin;
    let mut title = "Evidence report".to_string();
    let mut json_out: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let next = |i: &mut usize| -> Option<String> {
            *i += 1;
            args.get(*i).cloned()
        };
        match args[i].as_str() {
            "--rules" => rules_file = next(&mut i),
            "--admit" => admit = next(&mut i),
            "--title" => title = next(&mut i).unwrap_or(title.clone()),
            "--json" => json_out = next(&mut i),
            "--semiring" => {
                semiring = match next(&mut i).as_deref() {
                    Some("boolean") => SemiringChoice::Boolean,
                    Some("maxmin") => SemiringChoice::MaxMin,
                    Some("probability") => SemiringChoice::Probability,
                    other => {
                        eprintln!("unknown semiring {other:?} (boolean|maxmin|probability)");
                        return ExitCode::from(2);
                    }
                }
            }
            f if !f.starts_with("--") => files.push(f.to_string()),
            other => {
                eprintln!("unknown flag {other}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    if files.is_empty() {
        eprintln!(
            "usage: reed-report <evidence.json>... [--rules FILE] [--admit PRED] [--semiring S] [--title T] [--json OUT]"
        );
        return ExitCode::from(2);
    }

    let mut records: Vec<Evidence> = Vec::new();
    for f in &files {
        let text = match fs::read_to_string(f) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: cannot read '{f}': {e}");
                return ExitCode::FAILURE;
            }
        };
        match serde_json::from_str::<Vec<Evidence>>(&text) {
            Ok(mut batch) => records.append(&mut batch),
            Err(e) => {
                eprintln!("error: '{f}' is not an evidence-record array: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    let rules = match &rules_file {
        None => String::new(),
        Some(f) => match fs::read_to_string(f) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: cannot read rules '{f}': {e}");
                return ExitCode::FAILURE;
            }
        },
    };
    // --rules without --admit means the conventional predicate; neither
    // means the no-constriction baseline where tiers do all the sorting.
    let admit = admit.or_else(|| rules_file.as_ref().map(|_| "reportable".to_string()));

    let verdicts = match judge(
        &records,
        &Config {
            bases: &[],
            rules: &rules,
            admit: admit.as_deref(),
            semiring,
            semantics: Semantics::DfQuad,
        },
    ) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(out) = &json_out {
        match serde_json::to_string_pretty(&verdicts) {
            Ok(j) => {
                if let Err(e) = fs::write(out, j) {
                    eprintln!("error: cannot write '{out}': {e}");
                    return ExitCode::FAILURE;
                }
            }
            Err(e) => {
                eprintln!("error: serializing verdicts: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    print!("{}", report::render(&records, &verdicts, &title));
    ExitCode::SUCCESS
}
