# ADR 0003 — MIT license with a brand NOTICE file

**Status:** Accepted

## Context

EKO is being built as an open-source audiophile music player. The question of
license carries real consequences for adoption, forking, and moat.

Candidate options considered:

| License | Summary |
|---|---|
| GPL v2 / v3 | Copyleft — any derivative must also be open. Chosen by VLC, Audacity. Maximises code sharing but restricts commercial derivatives. |
| LGPL | Copyleft for the library itself; application layer may be proprietary. Common for audio libraries. |
| MIT / Apache-2.0 | Permissive — anyone can fork, use commercially, keep changes private. Maximises adoption. |
| BSL / SSPL | Source-available with commercial use restrictions. Common in infrastructure; unusual for desktop apps. |
| Proprietary | Closed source. No community adoption. |

EKO's moat is not the code. The code will be reproducible by any competent
developer. The moat is:

1. **The audio engine's bit-perfect decisions** — documented openly; replicating
   the engine is work but not a secret.
2. **The design taste** — the neumorphic Braun aesthetic, the Concept G signal
   path, the typography and motion. This is the maintainer's differentiator and it is
   visible in the compiled app; the CSS is not a secret.
3. **The brand** — EKO, the visual identity, the "best-in-class audiophile
   player" positioning.
4. **A potential hardware product** — a physical player styled to the software's
   aesthetic. Hardware cannot be open-sourced away.

Restricting the code with copyleft would not protect any of these moats. It
would reduce adoption (fewer users, fewer contributors, fewer mentions) and deter
the hardware OEM partnerships or integration scenarios that may emerge later.

The risk with a fully permissive license is that a commercial player forks EKO,
ships it with the same branding, and competes directly. A NOTICE file that is
part of the license requirement addresses this: any distribution must reproduce
the notice, which asserts the EKO name and brand ownership. This does not prevent
forking or commercial use, but it makes deliberate brand confusion legally messy
without imposing copyleft obligations on the code.

## Decision

License EKO under **MIT**. Include a `NOTICE` file in the repository root that:

- States EKO's authorship and the reactivepixels/EKO brand identity.
- Asserts that the EKO name, logotype, and visual design language are not covered
  by the MIT license and may not be used in derivatives without written permission.
- Is referenced from the MIT `LICENSE` file to make its inclusion a condition of
  distribution.

The code is free. The brand is not.

## Consequences

**Positive:**
- Maximum adoption: anyone can use the engine, build integrations, contribute
  without copyleft obligations.
- Audiophile communities (which value open code and scrutinisable signal paths)
  will engage more readily with a permissive license.
- Contributors retain their standard expectations about code reuse.
- The NOTICE mechanism is widely understood and respected; brand confusion via
  deliberate misuse is a trademark matter separately from the license.

**Negative / trade-offs:**
- MIT does not prevent a fork from taking EKO's entire codebase, stripping the
  NOTICE, and shipping a competing player. Enforcement of the NOTICE requirement
  depends on the integrity of the ecosystem, not legal lock-in.
- If EKO ever pivots toward a dual-license or commercial model, MIT-licensed code
  already distributed cannot be "un-licensed". Future commercial versions would
  need to be built on top of, or clearly separated from, the open codebase.
