//! Render verdicts as a report a human signs off on.
//!
//! The tiers are the point. A flat list of findings buries the one
//! distinction an auditor needs — what is INDEPENDENTLY VERIFIED versus
//! merely asserted versus actively dismissed — so the report leads with it:
//!
//! 1. **Reportable, verified** — admitted by the rules AND carrying an
//!    upheld check. The tier a compliance statement cites.
//! 2. **Reportable, unverified** — admitted, but nothing independent
//!    corroborated it. Real work items; weaker standing.
//! 3. **Contested** — admitted, but inhibiting bearings (an instructor
//!    exception, a counter-finding) argue against it. Shown with its
//!    contestedness so a reader sees HOW disputed, not just that it is.
//! 4. **Excluded** — refuted by a check, or not admitted by the rules.
//!    Kept in the report, because a dismissed finding with its reason is
//!    audit evidence too; a silently dropped one is a liability.
//!
//! Admission proofs are included verbatim: the report's claims about
//! itself are re-derivable, which is the family's bar for everything.

use std::collections::HashMap;

use evidence_core::Evidence;

use crate::Verdict;

/// Render a Markdown report from judged records. `records` supplies the
/// claims, checks, and dates the verdicts refer to; entries without a
/// matching record are rendered from the verdict alone.
pub fn render(records: &[Evidence], verdicts: &[Verdict], title: &str) -> String {
    let by_id: HashMap<&str, &Evidence> = records
        .iter()
        .map(|r| (r.evidence_id.as_str(), r))
        .collect();

    // Contested = evidence argues against it. Two distinct shapes: mixed
    // evidence (contestedness > 0, promotion AND inhibition present) and
    // one-sided inhibition (nothing promotes, so entropy is 0 but the
    // strength sits below the neutral 0.5). Both belong in this tier.
    let contested = |v: &Verdict| v.contestedness > 0.0 || v.strength < 0.5;
    let verified = |v: &Verdict| {
        by_id
            .get(v.on.as_str())
            .is_some_and(|r| r.check.as_ref().is_some_and(|c| c.upheld))
    };
    let refuted = |v: &Verdict| {
        by_id
            .get(v.on.as_str())
            .is_some_and(|r| r.check.as_ref().is_some_and(|c| !c.upheld))
    };

    let mut tiers: [Vec<&Verdict>; 4] = [vec![], vec![], vec![], vec![]];
    for v in verdicts {
        let tier = if !v.admitted || refuted(v) {
            3
        } else if contested(v) {
            2
        } else if verified(v) {
            0
        } else {
            1
        };
        tiers[tier].push(v);
    }
    // Strongest first within each tier; ties keep input order (stable sort).
    for tier in &mut tiers {
        tier.sort_by(|a, b| {
            b.strength
                .partial_cmp(&a.strength)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    let mut md = String::new();
    md.push_str(&format!("# {title}\n\n"));
    md.push_str(&format!(
        "{} claim(s) judged — {} reportable-verified, {} reportable-unverified, {} contested, {} excluded\n\n",
        verdicts.len(),
        tiers[0].len(),
        tiers[1].len(),
        tiers[2].len(),
        tiers[3].len()
    ));

    let names = [
        (
            "Reportable — independently verified",
            "Admitted by the rules and carrying an upheld check. The tier a compliance statement cites.",
        ),
        (
            "Reportable — unverified",
            "Admitted, but nothing independent corroborated it. Real work items with weaker standing.",
        ),
        (
            "Contested",
            "Admitted, but evidence argues against it — an exception or counter-finding. Contestedness says how disputed.",
        ),
        (
            "Excluded",
            "Refuted by a check or not admitted by the rules. Kept with reasons: a silently dropped finding is a liability.",
        ),
    ];
    for (i, (name, blurb)) in names.iter().enumerate() {
        if tiers[i].is_empty() {
            continue;
        }
        md.push_str(&format!("## {name}\n\n> {blurb}\n\n"));
        for v in &tiers[i] {
            let claim = by_id
                .get(v.on.as_str())
                .map(|r| r.claim.as_str())
                .unwrap_or(v.on.as_str());
            md.push_str(&format!("### {}\n\n", claim));
            md.push_str(&format!("- **Id:** `{}`\n", v.on));
            if let Some(r) = by_id.get(v.on.as_str()) {
                if let Some(c) = &r.check {
                    let ruling = if c.upheld { "upheld" } else { "REFUTED" };
                    md.push_str(&format!(
                        "- **Check:** {ruling} by {} ({:?})\n",
                        c.by, c.method
                    ));
                    if let Some(reason) = &c.reason {
                        md.push_str(&format!("  - {reason}\n"));
                    }
                }
                if let Some(at) = &r.generated_at_time {
                    md.push_str(&format!("- **Recorded:** {at}\n"));
                }
            }
            md.push_str(&format!(
                "- **Strength:** {:.3} · **Contestedness:** {:.3}\n",
                v.strength, v.contestedness
            ));
            if let Some(tag) = &v.admission {
                md.push_str(&format!("- **Admission:** {tag}\n"));
            }
            if let Some(proof) = &v.proof {
                md.push_str("- **Why admitted:**\n\n```\n");
                md.push_str(proof.trim_end());
                md.push_str("\n```\n");
            }
            md.push('\n');
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Semantics, SemiringChoice, judge};
    use evidence_core::{Bearing, Check, CheckMethod, Producer};

    /// a11y-agent-shaped records: a corroborated finding, an unchecked one,
    /// a refuted one, and an instructor exception inhibiting the first.
    fn records() -> Vec<Evidence> {
        let mut corroborated = Evidence::new(
            "u::1.4.3::e3",
            Producer::new("a11y-agent", "t"),
            "contrast 2.1:1 below 4.5:1",
        );
        corroborated.check = Check::corroborated(["measured", "vision"]);
        corroborated.generated_at_time = Some("2026-08-02T00:00:00Z".into());

        let unchecked = Evidence::new(
            "u::1.1.1::e9",
            Producer::new("a11y-agent", "t"),
            "image lacks a text alternative",
        );

        let mut refuted = Evidence::new(
            "u::4.1.2::e12",
            Producer::new("a11y-agent", "t"),
            "button has no accessible name",
        );
        refuted.check = Some(Check {
            method: CheckMethod::Adversarial,
            by: "stub".into(),
            upheld: false,
            reason: Some("named via aria-labelledby".into()),
        });

        let mut exception = Evidence::new(
            "instructor-exception-7",
            Producer::new("host", "t"),
            "decorative banner, exception granted",
        );
        exception.bearings = vec![Bearing::inhibits("u::1.4.3::e3", 0.6).with_basis("exception")];

        vec![corroborated, unchecked, refuted, exception]
    }

    fn config() -> Config<'static> {
        Config {
            bases: &[],
            rules: "reportable(F) :- verified(F).\nreportable(F) :- refuted(F).",
            admit: None, // set per test
            semiring: SemiringChoice::Boolean,
            semantics: Semantics::DfQuad,
        }
    }

    #[test]
    fn the_verified_only_tier_is_one_rule() {
        let mut cfg = config();
        cfg.rules = "reportable(F) :- verified(F).";
        cfg.admit = Some("reportable");
        let verdicts = judge(&records(), &cfg).unwrap();
        let admitted: Vec<_> = verdicts.iter().filter(|v| v.admitted).collect();
        assert_eq!(admitted.len(), 1, "{verdicts:?}");
        assert_eq!(admitted[0].on, "u::1.4.3::e3");
        assert!(admitted[0].proof.as_ref().unwrap().contains("verified("));
    }

    #[test]
    fn bearing_less_findings_are_judged_and_exceptions_are_not() {
        let verdicts = judge(&records(), &config()).unwrap();
        let ids: Vec<_> = verdicts.iter().map(|v| v.on.as_str()).collect();
        assert!(
            ids.contains(&"u::1.1.1::e9"),
            "standalone finding is a claim"
        );
        assert!(
            !ids.contains(&"instructor-exception-7"),
            "an arguer is not a claim: {ids:?}"
        );
    }

    #[test]
    fn the_report_tiers_findings_by_standing() {
        let mut cfg = config();
        cfg.rules = "";
        cfg.admit = None; // baseline: everything admitted, tiers do the sorting
        let verdicts = judge(&records(), &cfg).unwrap();
        let md = render(&records(), &verdicts, "Audit — course 1");

        // The only verified finding is contested by the exception, so the
        // verified tier is empty and its heading is (correctly) absent.
        assert!(
            !md.contains("## Reportable — independently verified"),
            "{md}"
        );
        let unverified_at = md.find("## Reportable — unverified").unwrap();
        let contested_at = md.find("## Contested").unwrap();
        let excluded_at = md.find("## Excluded").unwrap();
        assert!(unverified_at < contested_at && contested_at < excluded_at);

        // The exception drags the corroborated finding into Contested —
        // still standing, visibly disputed.
        let contested_block = &md[contested_at..excluded_at];
        assert!(contested_block.contains("u::1.4.3::e3"), "{md}");
        // The refuted finding is excluded WITH its reason kept.
        assert!(md[excluded_at..].contains("aria-labelledby"), "{md}");
        // The unchecked finding sits in the unverified tier.
        assert!(md[unverified_at..contested_at].contains("u::1.1.1::e9"));
    }
}
