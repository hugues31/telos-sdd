---
name: telos-brainstorm
description: Explore an early product or feature idea with structured divergence and convergence before creating a Telos intent. Use when the problem, opportunity, scope, or solution space is still uncertain; do not use when a sealed intent already defines the outcome.
---

# Telos Brainstorm

Use brainstorming to expose choices and assumptions. Do not turn the selected idea directly into code or a specification.

1. Run `telos brainstorm start --mode <mode>`. Use `choose` when the user knows the technique, `recommend` when the problem shape is clear, `random` to break fixation, or `progressive` for broad exploration followed by convergence.
2. Open the created `.telos/brainstorms/*.md` artifact.
3. Apply the recorded engine and seed. Preserve the seed so a random selection is reproducible.
4. Separate divergence from evaluation. Generate materially different ideas before ranking any of them.
5. Challenge assumptions, affected actors, second-order effects, failure cases, constraints, and evidence.
6. Fill `Promotion candidate` with one explicit candidate only after the user chooses it. Keep `None.` if nothing deserves promotion.
7. Hand the selected candidate to `$telos-intent`. Never silently promote it.

Available engines are SCAMPER, reverse brainstorming, six thinking hats, assumption reversal, morphological matrix, Jobs to be Done, pre-mortem, first principles, constraint removal, analogical transfer, worst possible idea, and impact/effort convergence.

Return the artifact path, chosen engine, strongest alternatives, rejected assumptions, and the promotion decision.

