# reed

**The judge for verifiable-evidence graphs.** Producers emit
[evidence-core](https://github.com/rustforrecess/evidence-core) records whose
bearings promote and inhibit claims; reed scores that graph into verdicts —
and every verdict carries the derivation that produced it.

Apache-2.0. On a loom, the reed beats each weft thread into place after the
heddles choose what can interlace: it is the part that decides what the cloth
actually holds.

## Two layers, deliberately separate

**Logic decides *whether*.** Caller-supplied Datalog rules run under a
provenance semiring ([sequent](https://github.com/rustforrecess/sequent)
Tier 3½). Constriction — *"admissible only with support from both the
symbolic and the vector side"* — is one rule:

```prolog
admissible(P) :- found(symbolic, P), found(vector, P).
```

The semiring chooses the reading: `Boolean` (hard admission), `MaxMin`
(admitted at the strength of its weakest link), `Probability` (product /
noisy-or). The admitting derivation comes back rendered, tags and all.

**Semantics decides *how much*.** The bipolar bearing graph is scored over
the admitted claims by DF-QuAD or log-odds, yielding a **strength** and a
**contestedness** — the entropy of the promote/inhibit split, because
0.6-from-unanimous-weak and 0.6-from-strong-conflict are different epistemic
situations one number cannot distinguish.

## Everything is an experimental variable

The design exists for ablation: record every signal once, then score the
same stored records under every condition — no re-retrieval.

| Knob | Values | Toggles |
| --- | --- | --- |
| `bases` | `found`, `informed-silence`, `stance`, … | which signal classes count |
| `rules` + `admit` | Datalog / `None` | constriction on/off, any conjunctive-support policy |
| `semiring` | `Boolean`, `MaxMin`, `Probability` | how the logical layer grades |
| `semantics` | `DfQuad`, `LogOdds` | how the bipolar layer aggregates |

```rust
let verdicts = reed::judge(&records, &reed::Config {
    bases: &["found", "informed-silence"],
    rules: "admissible(P) :- found(symbolic, P), found(vector, P).",
    admit: Some("admissible"),
    semiring: reed::SemiringChoice::MaxMin,
    semantics: reed::Semantics::DfQuad,
})?;
// verdicts[i]: { on, admitted, admission, strength, contestedness, proof }
```

The input is any set of evidence-core records with bearings. heddle's
`retrieve-with-evidence` testimony records are the first producer: their
`found` / `informed-silence` bearings and path kinds flatten directly into
the rule vocabulary.

## Status

v0.1 — the two-layer core, with tests covering constriction (weakest-link
admission, vector-only exclusion), the no-constriction baseline, ablation by
basis (silence costs strength and raises contestedness), and cross-semantics
sanity. Not yet a wasm component; the core is pure and dependency-light on
purpose, so that step mirrors heddle's when it comes.

Scallop (MIT) is the nearest prior system for the semiring layer; per
sequent's PRIOR-ART.md it is used as a behavioral oracle only — its source
is never read.
