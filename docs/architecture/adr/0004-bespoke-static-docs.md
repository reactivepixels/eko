# ADR 0004 — Docs site: bespoke static HTML (Astro Starlight tried, then dropped)

**Status:** Accepted (supersedes the initial "use Astro Starlight" decision)

## Context

EKO's documentation + marketing site has a hard constraint: **the design is part of
the product**. The neumorphic Braun aesthetic — Porcelain/Graphite themes, accent
orange `#ef6a1e`, dual soft-shadow tokens, the "precision instrument" feel — must
carry into the site. A site that looks like a generic doc template undermines the
whole signal. The owner's directive was explicit: **no compromise on the existing
design.** The taste baseline is the hand-built `concepts/docs.html` mockup, which is
the app's design system used directly.

We first chose **Astro Starlight**, planning to reach the neumorphic look by
overriding Starlight's CSS custom properties. That decision included an explicit
escape hatch: *if the framework constrains the design, drop to plain Astro / our own
layout — design wins.*

## What actually happened

Starlight was implemented and themed via a token-override stylesheet. The result was
**"EKO-coloured Starlight," not EKO** — Starlight's layout, chrome, typography, and
component shapes are generic, and swapping color/font *variables* doesn't make them
neumorphic. Matching the design would have meant fighting or forking the framework's
components. That is exactly the compromise the directive forbade, so the escape hatch
was triggered.

## Decision

**Drop Starlight. Ship the docs/marketing site as plain static HTML** (`site/`), built
directly on the app's design system (`neu.css` tokens, inline, no framework):

- `site/index.html` — landing
- `site/docs.html` — documentation (neumorphic sidebar, scroll-spy, themed code blocks,
  callouts, an embedded live "EKO Web Lite" player via `<iframe>`)
- `site/web-player.html` — the embeddable player
- `site/mobile.html` — the parked iOS concept

No build step, no dependencies, no framework chrome — it opens in a browser and deploys
as-is. The design is therefore 100% ours and pixel-matched to the app.

## Consequences

**Positive:**
- Zero design compromise — the site *is* the app's design language.
- No build, no dependency churn, no framework upgrade risk. Deploys to any static host
  (target: **eko.reactivepixels.com** on Vercel; GitHub Pages is a fine fallback).
- Trivial to keep in lockstep with the app's `neu.css` (same tokens, copied/shared).

**Negative / trade-offs:**
- We hand-roll what Starlight gave for free: nav, the active-section scroll-spy (done in
  ~10 lines of vanilla JS), and search. **Search is not implemented** — acceptable at this
  size; revisit (e.g. Pagefind as a static add-on) only if the docs grow large.
- Multi-page routing is manual (plain files + relative links) rather than framework-managed.
- Content lives in HTML, not Markdown/MDX — slightly less convenient to edit, but the
  payoff (total design control) is the whole point here.

## Lesson

For a product where design *is* the value proposition, reach for "framework + theme" only
if the framework's **structure** (not just its colors) already fits. When in doubt, build
the bespoke thing — it was less work than fighting the framework would have been.
