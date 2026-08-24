/**
 * The **free** `SubSong` → `Track` boundary: `toTrack` in `useSubsonic.ts`, which serves the
 * library, search and playlists.
 *
 * The assertions themselves live in `./toTrackContract.ts` — see that file for what is being
 * pinned and why (one operator: `||` and not `??`, or seven downstream placeholders render
 * blank). They are shared because there is a second implementation of the same contract,
 * `subSongToTrack` in `pro/useSmartPlaylistStore.ts`, and the two must not drift.
 *
 * **This file must stay importable with `src/pro/` absent.** The publish pipeline rsyncs the
 * free repo excluding `src/pro/` but keeping every other test file, so a relative import of the
 * Pro twin from here — which is what this file used to do — ships a broken test to the public
 * MIT repo and breaks its `npm run typecheck` and `vitest`. The Pro half of the contract is
 * asserted by `src/pro/subSongToTrack.test.ts`, which disappears with the tree it belongs to.
 *
 * Pure — no Tauri, no store, in keeping with the other suites here.
 */

import { toTrack } from "./useSubsonic";
import { describeNormaliserContract } from "./toTrackContract";

describeNormaliserContract("toTrack (free — library, search, playlists)", toTrack);
