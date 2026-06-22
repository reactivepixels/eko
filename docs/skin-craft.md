# EKO Skin Craft — building a skin's components without the recurring traps

The third leg of the skin docs. The other two answer *what* a skin is:

- `skin-architecture.md` — the **seam**: shared headless logic ↔ theme-owned pixels.
- `THEME-BLUEPRINT.md` — the **process**: concept-first, the 13-surface checklist, the gates.

This doc answers the question that actually ate the time: **how do you build the
skeuomorphic components themselves without the CSS stacking, shadow, and verification
traps that bit every agent who touched this player?** It is the construction manual.

> If you are about to theme this player and you read only one thing, read §1 and §5.

---

## 1. Why theming this player keeps stumbling (name it, then it stops)

Studio is **skeuomorphic**. That single fact is the source of every recurring problem:

1. **Every control is a 4–6 layer CSS stack, not one element.** A Studio knob is
   `gauge SVG + convex body + flat cap + cap inset-ring + dial + indicator` — six layers
   sharing one 88px box. A button is `puck + flat top + icon`. A switch is
   `track + travelling nub`. The realism *is* the layering.
2. **The bugs live in how those layers stack and cast shadows** — `z-index`, `transform`,
   `isolation`, and shadow-bleed near panel seams. These are **invisible in the code** and
   appear **only in rendered pixels**. You cannot review them by reading CSS.
3. **Token-only theming cannot express any of this** (proven twice — see
   `skin-architecture.md §1`), so you are *forced* to hand-build the layered components,
   which is exactly where traps 1–2 live.
4. **The live `file://` browser tab silently serves stale CSS**, so a real fix looks like
   "nothing changed" and a non-fix looks done. This produced most of the "nothing has
   changed / something is fishy" churn.

None of this is a skill gap — it's a **missing construction spec + a missing verification
loop**. This doc is both. Once the component recipes and the screenshot loop are in place,
skin work is fast and boring, which is the goal.

---

## 2. What a skin component actually is

A skin component = **a fixed layer stack + a material**. The *stack* (how many layers, in
what order, with what stacking rules) is **shared craft** — it is the same for every skin and
is specified in §3. The *material* (colours, gradients, shadow softness, radii) is **per-skin**
and lives in tokens (§4). You re-skin by swapping the material over the same proven stack —
**never** by reinventing the stack.

This is the resolution of "is a skin just tokens?" — **No. A skin is tokens applied over
theme-owned layered components.** The components are owned per skin but built from the same
small library of proven stacks below.

---

## 3. The proven stacks (canonical recipes)

Each recipe is the **minimum correct layering** plus the **one rule** that keeps it from
breaking. Values shown are Studio's; the *structure* is the reusable part.

### 3.1 Knob (the hardest, and the canary for everything)

Layers, back-to-front, all `position:absolute` inside a `position:relative` square
(`width:N; aspect-ratio:1`):

| Layer | Role | Key rule |
|-------|------|----------|
| `.arc` (SVG) | gauge: dark track + accent value arc | `overflow:visible` only if the arc has a glow filter; rotate the SVG, not the page |
| `.body` | convex base ring visible around the cap | **`z-index:-1`** — see THE KNOB LAW |
| `.top` | flat cap face | no z-index — natural order |
| `.top::after` | inset ring on the cap | optional; hide on dock/EQ variants |
| `.dial` | rotating carrier (`transform:rotate(θ)`) | holds the indicator; rotation lives here, nowhere else |
| `.ind` | indicator (dot for volume, recessed line for EQ) | child of `.dial` so it rotates with it |

**THE KNOB LAW.** The convex `.body` must paint **behind** the flat `.top` and `.dial`, or it
buries the indicator and the cap looks domed (a spherical top — the exact thing the owner
rejected). The fix that works and keeps the cap **flat** is **`.body{ z-index:-1 }`** — natural
tree order for everything else. Do **not** instead push `.top`/`.dial` to positive z-index:
that un-buries the dot *but domes the cap* (it changes which radial-gradient wins). Verified
both ways with screenshots.

- **Indicator variants are a scoped override, not a new knob.** EQ uses a recessed *line*
  (`.eq .knob .ind{ width:3.5px; height:12px; …recessed inset shadow }` + `.top::after{display:none}`);
  volume uses a *dot*. Same six-layer stack underneath.

### 3.2 Button / transport puck

`puck (raised) > flat top > icon`. Pressed state must **recess into the panel**, not just
nudge down — match the concept's `:active` exactly (inner shadow flips, top scales ~.99, icon
scales ~.92). A `translateY(2px)` with no inner-shadow change reads as a flat web button, not a
physical key, and was flagged as wrong. Keep one `.sm` size variant for secondary transport
(shuffle/repeat) rather than a second component.

### 3.3 Switch / slide toggle

`track (recessed well) > nub (raised, travels)`. The nub travels with `transform:translateX`
on an `.on` class; the accent fills the track behind it. Never animate `left` (jank); always
`transform`.

### 3.4 Recessed well / screen (search field, spectrum, track lists)

A surface pressed *into* the panel: `background:var(--face-lo)` + the shared inset recess token
(`--in`). The spectrum and track lists are "screens" = recessed wells with content on top.

### 3.5 Framed print (album covers)

A single rounded box with the matte mat painted by `::after` **over** the art
(`inset 0 0 0 4px var(--face)`), so corners never expose raw colour and the art reads as a
framed print. Now-playing badge and catalogue label sit at `z-index:3` above the mat.

### 3.6 LED meter

Rounded recessed well + N segment children that light from the accent. Lives in the dock's EQ
area — **the meter belongs to the mini-equalizer region; do not relocate or delete it** (owner
correction). The now-playing deck does **not** get a second redundant meter.

---

## 4. Token layering (structural vs material)

Two tiers. Keep them separate or skins leak into each other.

- **Structural primitives (shared shape):** the recess token `--in`, the raised-cap token
  `--raise`, layer insets (`8% / 16% / 20%`), z-order rules. These are the *stack*, shared by
  every skin. Do not fork them per skin.
- **Material values (per skin):** `--face`, `--face-lo`, warm shadow rgb `--sh` (Studio: warm
  `62,58,50`, **never** pure black), ink, mono `--label`, accent. These are what `data-skin`
  / `data-theme` swap. Light **and** dark are both material sets of the same structure.

Rule of thumb: if changing a value changes the *shape or depth model*, it is structural and
shared; if it only changes *colour or softness*, it is material and per-skin.

---

## 5. Verification protocol (non-negotiable — this killed most of the churn)

**Never trust the live `file://` tab to reflect a CSS edit, and never claim a visual fix you
have not seen in a fresh render.** The loop:

```bash
B="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
# Fresh headless render — bypasses the stale live tab entirely.
"$B" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=2 \
  --window-size=1320,900 --screenshot=/tmp/v.png \
  "file://$PWD/concepts/studio-player.html?v=now"     # ?v=<view> drives which screen renders
# Zoom a region to inspect a detail (ffmpeg crop = W:H:X:Y on the 2x image):
ffmpeg -y -i /tmp/v.png -vf "crop=700:560:1960:1130,scale=iw*1.15:ih*1.15" /tmp/zoom.png
```

Then **Read the PNG** and look. For the owner's live tab, re-open cache-busted after every
edit so he never refreshes manually: `open "file://$PWD/concepts/studio-player.html?v=$(date +%s)"`.

**Diagnosing an invisible artifact (the method that cracked the volume-knob box):**
1. **`elementFromPoint(x,y)`** at the artifact via `--dump-dom` + a `setTimeout` that writes the
   hit element into `document.title`. Tells you *what is actually there* vs what you assume.
2. **Subtract layers.** Hide the suspected element (`visibility:hidden`), re-render. Gone →
   it's that element (or its shadow). Still there → keep looking. This converts a guessing game
   into a binary search.
3. **Isolate the property.** Once you have the element, kill one property at a time
   (the shadow, the filter, the z-index) until the artifact moves. Then you know the cause, not
   a guess.

---

## 6. Stacking-context cheatsheet (the facts that bit us)

- A `position`ed element with `z-index:auto` paints in **tree order** — later siblings on top.
  This is why `.body` (earlier in the DOM) could still bury `.top`: a sibling's stacking made
  the naive order wrong. `z-index:-1` on `.body` is the reliable fix.
- `transform` creates a **stacking context** *and* becomes the containing block for
  fixed/absolute descendants. The `.dial`'s `rotate` is fine because the indicator is its child;
  do not put `transform` on a parent whose descendants must escape it.
- `isolation:isolate` creates a stacking context **without** painting changes — useful to
  *contain* a negative-z child, but it does **not** clip shadow that already extends outside the
  box. (Tested on the knob: it did not fix the dock box, because the box was shadow-framing, not
  a z-index escape. Don't reach for `isolation` reflexively — diagnose first per §5.)
- **⚠️ FIRST CHECK, ALWAYS: what rules actually match this element?** Before any pixel work,
  screenshot, or shadow theory — open DevTools → Styles on the element (or grep the selector) and
  read the *whole* matched cascade. This is the single step that finds most "mystery" visual bugs
  in seconds, and skipping it is how a trivial bug eats a day.
- **Worked example — the "phantom plate" (a class-name collision).** The volume knob showed a
  lighter rounded-rectangle "plate" behind it. The real cause was embarrassingly simple: the knob
  was `<div class="knob dock">`, and that bare **`dock` modifier collided with the transport
  component's `.dock` rule** (`background: linear-gradient(...); box-shadow: ...`). So the knob
  *inherited the dock's background gradient and box-shadow* — the "plate" was literally the dock's
  own background painted behind the knob. **Fix: rename the modifier** (`.knob.vk`) so it can't
  match the component class. One look at the matched-rules list shows `.dock` applying to a knob —
  obvious immediately.
  - **The anti-lesson (how this was botched).** It was mis-diagnosed *repeatedly* by going straight
    to pixels: blamed the knob's body shadow, then the dock's lift shadow, then GPU compositing
    layers — each "fix" claimed from a screenshot, none correct. Three traps to never repeat:
    1. **Never debug the symptom's pixels before reading its cascade.** The plate was *on* the knob
       but *authored* by a rule that didn't even name the knob — only the matched-rules list reveals
       that. Subtraction tests and diffs can't, because they operate on rendered output, not the
       source of truth (the cascade).
    2. **Bare modifier classes are a footgun.** `.knob.dock` reads as "knob, dock variant," but CSS
       sees *any element with class `dock`*. Namespace modifiers (`.knob--dock` / `.knob.vk`) or
       always pair them (`.knob.dock` only ever as a compound) — never let a component name double
       as a modifier.
    3. **Your render harness is not the user's renderer.** Headless software-rendered Chrome showed
       the low-contrast gradient as nearly invisible; the user's GPU Chrome showed it plainly. When
       your screenshots and the user disagree, the user is right — get ground truth from *their*
       browser (DevTools, a build marker) instead of trusting your harness.

---

## 7. Adapt, don't transcribe (the design-judgment rule)

The inspiration concepts (`skin-studio.html`, `skin-studio-library.html`) are the **material
and feel** reference, not a layout to copy pixel-for-pixel. The shipped concept
(`studio-player.html`) is **fully featured** — every Porcelain screen, nav path, and control —
rendered in Studio's material. Take the *language* (knob anatomy, press-in buttons, recessed
screens, mono micro-labels, warm shadow) and apply it intelligently to the fuller surface.
Removing features to match a simplified mockup is always wrong; so is mechanically cloning the
mockup's smaller control set. (Owner: *"You're taking the elements and the aesthetic inspiration
… and applying that to our new one."*)

---

## 8. The skin-build checklist (run this, in order, every skin)

1. **Gate 1 — aesthetic lock.** One static HTML concept, full feature set, in the new material.
   Owner-approved before any app code.
2. **Build each surface** (the 13 in `THEME-BLUEPRINT.md §4`) from the §3 stacks over the shared
   hooks. Own all pixels; share zero DOM.
3. **Verify every surface by screenshot** (§5), not by eye on the live tab, not by reading CSS.
4. **Run the trap sweep** before calling it done:
   - [ ] every knob: cap **flat**, indicator visible, gauge correct (§3.1)
   - [ ] every button: pressed state recesses *into* the panel (§3.2)
   - [ ] no phantom box/line where a control meets a panel seam (§6)
   - [ ] light **and** dark both rendered and checked (§4)
   - [ ] nothing relocated/removed vs the feature set; meter in its region (§3.6)
5. **Only then** wire it behind `data-skin` with Porcelain left pixel-identical.
