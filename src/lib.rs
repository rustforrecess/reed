//! **reed** — the judge for verifiable-evidence graphs.
//!
//! On a loom the reed beats each weft thread into place: after the heddles
//! choose what CAN interlace, the reed decides what the cloth actually
//! holds. This does the same for evidence: producers (heddle, mineweave,
//! a11y-agent) emit records whose bearings promote and inhibit claims;
//! reed scores that graph into verdicts — and every verdict carries the
//! derivation that produced it.
//!
//! Two layers, deliberately separate:
//!
//! 1. **Logic decides WHETHER** — caller-supplied Datalog rules run under a
//!    provenance semiring (sequent Tier 3½). Constriction ("admissible
//!    only with support from BOTH the symbolic and vector sides") is one
//!    rule; conjunctive support of any shape is a rule body; the semiring
//!    chooses hard admission (Boolean), weakest-link (MaxMin), or an
//!    independent-evidence reading (Probability).
//! 2. **Semantics decides HOW MUCH** — the bipolar bearing graph (promote /
//!    inhibit, weighted) is scored over the admitted claims by a gradual
//!    semantics (DF-QuAD or log-odds), yielding a strength AND a
//!    contestedness: how one-sided the evidence was, which a bare strength
//!    hides.
//!
//! Every knob is an experimental variable, because the whole point is
//! ablation: `bases` includes or excludes signal classes ("found",
//! "informed-silence", "stance"), `rules`+`admit` toggle constriction,
//! `semiring` and `semantics` swap the mathematics — all over the SAME
//! stored records, no re-retrieval per condition.

pub mod report;

use std::collections::HashMap;

use evidence_core::{Evidence, Polarity};
use sequent::prolog::Program;
use sequent::weighted::{Boolean, MaxMin, Probability, Semiring, saturate_weighted};

/// Which semiring the logical layer runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemiringChoice {
    Boolean,
    MaxMin,
    Probability,
}

/// How the gradual layer aggregates the bipolar graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Semantics {
    /// DF-QuAD (Rago et al.): support and attack each combine by
    /// `1 − ∏(1 − wᵢ)`, then move the base score toward 1 or 0 by the
    /// difference. Robust to correlated evidence; order-independent.
    DfQuad,
    /// Log-odds voting: `σ(logit(b) + Σw_promote − Σw_inhibit)` — the
    /// naive-Bayes reading, principled exactly insofar as testimonies are
    /// independent.
    LogOdds,
}

pub struct Config<'a> {
    /// Signal classes (bearing `basis` values) that count. Empty = all.
    pub bases: &'a [&'a str],
    /// Caller Datalog policy, e.g.
    /// `admissible(P) :- found(symbolic, P), found(vector, P).`
    /// Empty = no logical layer; everything is admitted.
    pub rules: &'a str,
    /// The predicate whose derivation admits a claim (usually
    /// `admissible`). `None` admits every claim — the no-constriction
    /// baseline condition.
    pub admit: Option<&'a str>,
    pub semiring: SemiringChoice,
    pub semantics: Semantics,
}

/// One judged claim.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    /// The `evidence_id` of the record whose claim was judged.
    pub on: String,
    /// Did the logical layer admit it? (Always true with `admit: None`.)
    pub admitted: bool,
    /// The admission tag, rendered, when a semiring computed one —
    /// e.g. `"0.7000"` under MaxMin: the weakest link of the strongest
    /// admitting derivation.
    pub admission: Option<String>,
    /// Gradual strength in [0,1] from the bipolar graph (only meaningful
    /// when admitted).
    pub strength: f64,
    /// How contested the evidence was, in [0,1]: 0 = one-sided, 1 =
    /// perfectly balanced promotion and inhibition. A strength without
    /// its contestedness overstates what is known.
    pub contestedness: f64,
    /// The admitting derivation, rendered with tags — absent when no
    /// rules ran or the claim was not admitted.
    pub proof: Option<String>,
}

/// Judge `records` under `config`.
///
/// A record ARGUES or IS ARGUED ABOUT: anything a bearing targets is a
/// claim, and any record carrying no bearings of its own is a claim too.
/// That one rule covers both producers without special cases — heddle's
/// testimonies (bearing-carriers) judge its passages, while a11y-agent's
/// findings (bearing-less, self-standing) are judged directly, and an
/// instructor-exception record that inhibits a finding automatically stops
/// being a claim and becomes an arguer.
pub fn judge(records: &[Evidence], config: &Config<'_>) -> Result<Vec<Verdict>, String> {
    let included = |basis: &Option<String>| {
        config.bases.is_empty() || basis.as_deref().is_some_and(|b| config.bases.contains(&b))
    };

    // The bipolar graph: per target, the included promoting / inhibiting
    // weights, in record order (deterministic folds).
    let mut promotes: HashMap<&str, Vec<f64>> = HashMap::new();
    let mut inhibits: HashMap<&str, Vec<f64>> = HashMap::new();
    let mut targets: Vec<&str> = Vec::new();
    for r in records {
        for b in &r.bearings {
            if !included(&b.basis) {
                continue;
            }
            if !promotes.contains_key(b.on.as_str()) && !inhibits.contains_key(b.on.as_str()) {
                targets.push(&b.on);
            }
            match b.polarity {
                Polarity::Promotes => promotes.entry(&b.on).or_default().push(b.weight),
                Polarity::Inhibits => inhibits.entry(&b.on).or_default().push(b.weight),
            }
        }
    }
    // Bearing-less records are claims in their own right (see above).
    let mut seen: std::collections::HashSet<&str> = targets.iter().copied().collect();
    for r in records {
        if r.bearings.is_empty() && seen.insert(r.evidence_id.as_str()) {
            targets.push(&r.evidence_id);
        }
    }

    // The logical layer, when rules are given: flatten testimony bearings
    // into `found(kind, target)` / `silent(kind, target)` facts — the
    // vocabulary constriction rules are written in — plus generic
    // `supports/attacks(source, target)` for non-testimony records.
    // Evidence ids are interned as e0, e1, ... because they are not
    // Datalog atoms; the table maps back.
    let admission = if config.rules.trim().is_empty() || config.admit.is_none() {
        None
    } else {
        Some(run_rules(records, config, &targets, &included)?)
    };

    let mut out = Vec::new();
    for target in targets {
        let (admitted, tag, proof) = match &admission {
            None => (true, None, None),
            Some(a) => match a.get(target) {
                Some((tag, proof)) => (true, Some(tag.clone()), Some(proof.clone())),
                None => (false, None, None),
            },
        };

        let p = promotes.get(target).cloned().unwrap_or_default();
        let i = inhibits.get(target).cloned().unwrap_or_default();
        let strength = if admitted {
            score(&p, &i, config.semantics)
        } else {
            0.0
        };

        out.push(Verdict {
            on: target.to_string(),
            admitted,
            admission: tag,
            strength,
            contestedness: contestedness(&p, &i),
            proof,
        });
    }
    Ok(out)
}

/// Gradual strength from a neutral base of 0.5 — the admission tag is
/// reported separately rather than folded in, so the two layers stay
/// independently ablatable.
fn score(promotes: &[f64], inhibits: &[f64], semantics: Semantics) -> f64 {
    const BASE: f64 = 0.5;
    match semantics {
        Semantics::DfQuad => {
            let vs = 1.0 - promotes.iter().fold(1.0, |acc, w| acc * (1.0 - w));
            let va = 1.0 - inhibits.iter().fold(1.0, |acc, w| acc * (1.0 - w));
            if vs >= va {
                BASE + (1.0 - BASE) * (vs - va)
            } else {
                BASE - BASE * (va - vs)
            }
        }
        Semantics::LogOdds => {
            let logit = |p: f64| (p / (1.0 - p)).ln();
            let z = logit(BASE) + promotes.iter().sum::<f64>() - inhibits.iter().sum::<f64>();
            1.0 / (1.0 + (-z).exp())
        }
    }
}

/// Shannon entropy of the promote/inhibit mass split: 0 when the evidence
/// all points one way, 1 when it is perfectly balanced. Reported alongside
/// strength because 0.6-from-unanimous-weak and 0.6-from-strong-conflict
/// are different epistemic situations a single number cannot distinguish.
fn contestedness(promotes: &[f64], inhibits: &[f64]) -> f64 {
    let s: f64 = promotes.iter().sum();
    let a: f64 = inhibits.iter().sum();
    let total = s + a;
    if total <= 0.0 || s == 0.0 || a == 0.0 {
        return 0.0;
    }
    let (p, q) = (s / total, a / total);
    -(p * p.log2() + q * q.log2())
}

/// Admitted targets → (rendered admission tag, rendered proof).
type Admission = HashMap<String, (String, String)>;

fn run_rules<F: Fn(&Option<String>) -> bool>(
    records: &[Evidence],
    config: &Config<'_>,
    targets: &[&str],
    included: &F,
) -> Result<Admission, String> {
    // Intern ids as atoms: evidence ids ("pass-1#p1") are not Datalog
    // atoms, so each becomes e0, e1, ... and the table maps back.
    let mut atom: HashMap<String, String> = HashMap::new();
    let intern = |id: &str, atom: &mut HashMap<String, String>| -> String {
        let n = atom.len();
        atom.entry(id.to_string())
            .or_insert_with(|| format!("e{n}"))
            .clone()
    };

    let mut facts: Vec<String> = Vec::new();
    let mut weights: HashMap<usize, f64> = HashMap::new();
    let push_fact =
        |facts: &mut Vec<String>, weights: &mut HashMap<usize, f64>, text: String, w: f64| {
            weights.insert(facts.len(), w);
            facts.push(text);
        };

    for r in records {
        // A testimony record names its path kind; its bearings become the
        // found/silent vocabulary. Anything else becomes supports/attacks.
        let kind = r
            .trace
            .as_ref()
            .filter(|t| t.kind == "path-testimony")
            .and_then(|t| t.steps.get("kind"))
            .and_then(|k| k.as_str().map(str::to_owned));
        for b in &r.bearings {
            if !included(&b.basis) {
                continue;
            }
            let tgt = intern(&b.on, &mut atom);
            match (&kind, b.polarity) {
                (Some(k), Polarity::Promotes) => push_fact(
                    &mut facts,
                    &mut weights,
                    format!("found({k}, {tgt})."),
                    b.weight,
                ),
                (Some(k), Polarity::Inhibits) => push_fact(
                    &mut facts,
                    &mut weights,
                    format!("silent({k}, {tgt})."),
                    b.weight,
                ),
                (None, Polarity::Promotes) => {
                    let src = intern(&r.evidence_id, &mut atom);
                    push_fact(
                        &mut facts,
                        &mut weights,
                        format!("supports({src}, {tgt})."),
                        b.weight,
                    )
                }
                (None, Polarity::Inhibits) => {
                    let src = intern(&r.evidence_id, &mut atom);
                    push_fact(
                        &mut facts,
                        &mut weights,
                        format!("attacks({src}, {tgt})."),
                        b.weight,
                    )
                }
            }
        }
    }

    // Check facts: a record's OWN verification status, so admission rules
    // can constrain on it — `reportable(F) :- verified(F).` is the whole
    // verified-only tier of an a11y report, and `refuted` lets rules
    // exclude findings an adversarial pass dismissed.
    for r in records {
        if let Some(c) = &r.check {
            let a = intern(&r.evidence_id, &mut atom);
            let fact = if c.upheld { "verified" } else { "refuted" };
            push_fact(&mut facts, &mut weights, format!("{fact}({a})."), 1.0);
        }
    }

    // Temporal facts: a record whose SOURCE carries a PROV-O
    // generatedAtTime becomes `dated(e, YEAR)`, so rules can constrain
    // with sequent's comparison builtins:
    //   current(P) :- dated(P, T), ge(T, 2020).
    // Year granularity on purpose for v0.1 — Datalog terms are integers,
    // and admission policies bind to editions ("standards current as of
    // 2020"), not timestamps. Finer grain can become dated(e, days) later
    // without a shape change.
    for r in records {
        let year = r
            .source
            .as_ref()
            .and_then(|s| s.generated_at_time.as_deref())
            .and_then(|t| t.get(..4))
            .and_then(|y| y.parse::<i64>().ok());
        if let Some(y) = year {
            let a = intern(&r.evidence_id, &mut atom);
            push_fact(&mut facts, &mut weights, format!("dated({a}, {y})."), 1.0);
        }
    }

    let src = format!("{}\n{}", facts.join("\n"), config.rules);
    let prog = Program::parse(&src)?;
    let admit = config.admit.expect("checked by caller");

    // One saturation per semiring; the tag type differs, so each arm
    // renders its own results into the common Admission shape.
    let mut admitted = Admission::new();
    let mut check = |render: &dyn Fn(&str) -> Option<(String, String)>| {
        for t in targets {
            if let Some(a) = atom.get(*t) {
                if let Some((tag, proof)) = render(a) {
                    admitted.insert((*t).to_string(), (tag, proof));
                }
            }
        }
    };
    match config.semiring {
        SemiringChoice::Boolean => {
            let w = saturate_weighted(&prog, &weights, &Boolean, 100_000, 1_000)?;
            check(&|a: &str| {
                let f = w.lookup(&prog, &format!("{admit}({a})"))?;
                Some((Boolean.render(w.tag(f)?), w.explain(f, &prog, &Boolean)))
            });
        }
        SemiringChoice::MaxMin => {
            let w = saturate_weighted(&prog, &weights, &MaxMin, 100_000, 1_000)?;
            check(&|a: &str| {
                let f = w.lookup(&prog, &format!("{admit}({a})"))?;
                Some((MaxMin.render(w.tag(f)?), w.explain(f, &prog, &MaxMin)))
            });
        }
        SemiringChoice::Probability => {
            let w = saturate_weighted(&prog, &weights, &Probability, 100_000, 1_000)?;
            check(&|a: &str| {
                let f = w.lookup(&prog, &format!("{admit}({a})"))?;
                Some((
                    Probability.render(w.tag(f)?),
                    w.explain(f, &prog, &Probability),
                ))
            });
        }
    }
    Ok(admitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evidence_core::{Bearing, Producer, Trace};
    use serde_json::json;

    /// A heddle-shaped testimony: path `kind` promoting/inhibiting targets.
    fn testimony(id: &str, kind: &str, bearings: Vec<Bearing>) -> Evidence {
        let mut e = Evidence::new(
            id,
            Producer::new("heddle", "test"),
            format!("{kind} testifies"),
        );
        e.trace = Some(Trace {
            kind: "path-testimony".into(),
            steps: json!({ "kind": kind }),
        });
        e.bearings = bearings;
        e
    }

    /// Both kinds found p1; only vector found p2, and symbolic (marked
    /// silence-informative) stayed silent about it.
    fn heddle_pass() -> Vec<Evidence> {
        vec![
            testimony(
                "t-sym",
                "symbolic",
                vec![
                    Bearing::promotes("p1", 0.7).with_basis("found"),
                    Bearing::inhibits("p2", 0.7).with_basis("informed-silence"),
                ],
            ),
            testimony(
                "t-vec",
                "vector",
                vec![
                    Bearing::promotes("p1", 0.9).with_basis("found"),
                    Bearing::promotes("p2", 0.9).with_basis("found"),
                ],
            ),
        ]
    }

    const CONSTRICTION: &str = "admissible(P) :- found(symbolic, P), found(vector, P).";

    #[test]
    fn constriction_admits_only_cross_kind_support_with_weakest_link() {
        let verdicts = judge(
            &heddle_pass(),
            &Config {
                bases: &["found"],
                rules: CONSTRICTION,
                admit: Some("admissible"),
                semiring: SemiringChoice::MaxMin,
                semantics: Semantics::DfQuad,
            },
        )
        .unwrap();
        let p1 = verdicts.iter().find(|v| v.on == "p1").unwrap();
        let p2 = verdicts.iter().find(|v| v.on == "p2").unwrap();
        assert!(p1.admitted);
        assert_eq!(p1.admission.as_deref(), Some("0.7000"), "min(0.7, 0.9)");
        assert!(
            p1.proof.as_ref().unwrap().contains("admissible(e"),
            "{p1:?}"
        );
        assert!(
            !p2.admitted,
            "vector-only support must not pass constriction"
        );
        assert_eq!(p2.strength, 0.0);
    }

    #[test]
    fn temporal_constriction_excludes_stale_sources() {
        // PROV-O dates on the passage records' sources become dated(P, Y)
        // facts; a rule bounds the edition year. p1 and p2 have identical
        // vector support — only their dates differ.
        let mut records = heddle_pass();
        let dated = |id: &str, t: &str| {
            let mut e = Evidence::new(id, Producer::new("host", "test"), format!("{id} relevant"));
            e.source = Some(evidence_core::Source {
                id: "standards".into(),
                locator: None,
                generated_at_time: Some(t.into()),
            });
            e
        };
        records.push(dated("p1", "2015-06-01T00:00:00Z"));
        records.push(dated("p2", "2024-09-01T00:00:00Z"));

        let verdicts = judge(
            &records,
            &Config {
                bases: &["found"],
                rules: "current(P) :- dated(P, T), ge(T, 2020).\n\
                        admissible(P) :- found(vector, P), current(P).",
                admit: Some("admissible"),
                semiring: SemiringChoice::MaxMin,
                semantics: Semantics::DfQuad,
            },
        )
        .unwrap();
        let p1 = verdicts.iter().find(|v| v.on == "p1").unwrap();
        let p2 = verdicts.iter().find(|v| v.on == "p2").unwrap();
        assert!(!p1.admitted, "2015 source fails the 2020 bound: {p1:?}");
        assert!(p2.admitted, "{p2:?}");
        assert!(
            p2.proof.as_ref().unwrap().contains("dated("),
            "the proof shows which date satisfied the bound: {p2:?}"
        );
    }

    #[test]
    fn without_constriction_everything_is_judged_on_strength_alone() {
        // The baseline condition of the ablation grid: same records,
        // admit: None — p2 comes back, carried by its vector support.
        let verdicts = judge(
            &heddle_pass(),
            &Config {
                bases: &[],
                rules: "",
                admit: None,
                semiring: SemiringChoice::Boolean,
                semantics: Semantics::DfQuad,
            },
        )
        .unwrap();
        let p2 = verdicts.iter().find(|v| v.on == "p2").unwrap();
        assert!(p2.admitted);
        assert!(p2.strength > 0.5, "promoted more than inhibited: {p2:?}");
    }

    #[test]
    fn excluding_a_signal_class_changes_the_verdict() {
        // The ablation switch itself: with informed-silence included, p2's
        // strength drops and its contestedness rises versus found-only —
        // from the SAME records, no re-retrieval.
        let with = |bases: &'static [&'static str]| {
            judge(
                &heddle_pass(),
                &Config {
                    bases,
                    rules: "",
                    admit: None,
                    semiring: SemiringChoice::Boolean,
                    semantics: Semantics::DfQuad,
                },
            )
            .unwrap()
            .into_iter()
            .find(|v| v.on == "p2")
            .unwrap()
        };
        let found_only = with(&["found"]);
        let both = with(&["found", "informed-silence"]);
        assert!(
            both.strength < found_only.strength,
            "silence must cost strength"
        );
        assert!(
            found_only.contestedness == 0.0,
            "one-sided evidence: {found_only:?}"
        );
        assert!(both.contestedness > 0.9, "near-balanced conflict: {both:?}");
    }

    #[test]
    fn both_semantics_agree_on_direction_and_differ_on_shape() {
        let run = |semantics| {
            judge(
                &heddle_pass(),
                &Config {
                    bases: &[],
                    rules: "",
                    admit: None,
                    semiring: SemiringChoice::Boolean,
                    semantics,
                },
            )
            .unwrap()
        };
        let df = run(Semantics::DfQuad);
        let lo = run(Semantics::LogOdds);
        for (a, b) in df.iter().zip(lo.iter()) {
            assert_eq!(a.on, b.on);
            // Same side of neutral, even where the curves differ.
            assert_eq!(a.strength > 0.5, b.strength > 0.5, "{a:?} vs {b:?}");
        }
    }
}
