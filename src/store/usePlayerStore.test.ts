/**
 * Store-level guard for the bit-perfect seal's inputs.
 *
 * The seal itself is derived in Rust (`eko_core::signal_path::derive`). What the store
 * owns is the *snapshot* it hands Rust — and the dominant input, `engineInfo`, is only
 * refreshed by the native poll, i.e. up to 120 ms behind a track change (longer for a
 * server stream, which reports nothing until the stream opens).
 *
 * So on a track change the store must DROP `engineInfo`. Blanking the derived seal is not
 * enough: `playAt` ends by calling `syncEq`/`syncEqMode`/`syncParamEq`/`syncVol`/
 * `applyReplayGain`, each of which triggers a re-derivation in the same synchronous tick.
 * With a stale `engineInfo` still in the store, the last of those wins and repaints the
 * OUTGOING track's verdict — so a 44.1 kHz track followed by a 96 kHz one the device will
 * resample would keep asserting BIT-PERFECT over an already-resampling stream.
 *
 * These tests fail if `engineInfo: null` is removed from `playAt`'s `set()`.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

/**
 * Stand-in for `eko_core::signal_path::derive`, faithful in the ONE respect these tests
 * turn on: a seal is only derivable once the engine has reported a stream. With stream
 * info present and nothing touching the samples it returns the real strongest claim, so a
 * test asserting the verdict was blanked can actually fail. Returning a blanket `null`
 * here — as this mock first did — made that assertion vacuous, because `undefined`
 * trivially differs from `"BIT-PERFECT"`.
 *
 * The real derivation's own correctness is covered in `crates/eko-core/src/signal_path.rs`.
 */
function fakeDerive(args: unknown): unknown {
  const info = (args as { input?: { info?: unknown } })?.input?.info;
  if (!info) {
    return {
      active: false,
      pure: false,
      eqActive: false,
      attenuated: false,
      rgActive: false,
      resampled: false,
      osResampled: false,
      codec: "",
      src: "",
      output: "",
      engineLabel: "",
      sealLabel: "",
      rgLabel: "Off",
    };
  }
  return { ...BIT_PERFECT_SEAL };
}

// Every native call the store makes funnels through this one function.
const invoke = vi.fn(async (cmd: string, args?: unknown): Promise<unknown> => {
  if (cmd === "signal_path") return fakeDerive(args);
  // Rust owns the ReplayGain decision; the store must consume it, not compute one.
  if (cmd === "signal_replaygain") return { engineDb: null, sealDb: null };
  return null;
});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => invoke(...(a as [string, unknown])),
}));

import { usePlayerStore } from "./usePlayerStore";
import type { Track } from "../types";

const track = (id: string, path: string): Track => ({
  id,
  path,
  title: id,
  artist: "A",
  album: "B",
  duration: 100,
  bitrate: null,
  sampleRate: null,
  channels: 2,
});

/** The outgoing track's stream info: 44.1 kHz throughout, i.e. genuinely bit-perfect. */
const PREVIOUS_TRACK_INFO = {
  device: "Topping E30",
  rate: 44100,
  srcRate: 44100,
  devRate: 44100,
  bits: 16,
  codec: "flac",
  channels: 2,
};

/** The strongest claim the product makes — what the outgoing track legitimately showed. */
const BIT_PERFECT_SEAL = {
  active: true,
  pure: true,
  eqActive: false,
  attenuated: false,
  rgActive: false,
  resampled: false,
  osResampled: false,
  codec: "FLAC",
  src: "FLAC · 44.1 kHz · 16-bit",
  output: "Topping E30 · 44.1 kHz",
  engineLabel: "Bit-perfect",
  sealLabel: "BIT-PERFECT",
  rgLabel: "Off",
};

describe("the seal's inputs on a track change", () => {
  beforeEach(() => {
    invoke.mockClear();
    usePlayerStore.setState({
      tracks: [track("one", "/music/one.flac"), track("two", "/music/two.flac")],
      currentIndex: 0,
      engineActive: true,
      engineInfo: PREVIOUS_TRACK_INFO,
      // A seal already on screen, asserting the strongest possible claim.
      signalPath: { ...BIT_PERFECT_SEAL },
    });
  });

  afterEach(() => {
    // `playAt` starts the native poll and the spectrum timer; stop them so the intervals
    // don't outlive the test.
    usePlayerStore.getState().stop();
  });

  it("drops the outgoing track's stream info, so no seal can be derived from it", async () => {
    await usePlayerStore.getState().playAt(1);
    expect(usePlayerStore.getState().engineInfo).toBeNull();
  });

  it("keeps it dropped after playAt's DSP re-syncs have all run", async () => {
    await usePlayerStore.getState().playAt(1);
    // syncEq / syncEqMode / syncParamEq / syncVol / applyReplayGain have now each fired a
    // refresh. Let their promises settle — none may resurrect the previous stream info.
    await Promise.resolve();
    await Promise.resolve();
    expect(usePlayerStore.getState().engineInfo).toBeNull();
    expect(usePlayerStore.getState().engineInfo).not.toEqual(PREVIOUS_TRACK_INFO);
  });

  it("blanks the previous verdict rather than carrying BIT-PERFECT across the change", async () => {
    await usePlayerStore.getState().playAt(1);
    // Let every in-tick refresh settle. `fakeDerive` returns a real BIT-PERFECT seal
    // whenever it is handed stream info, so if any of them still sees the outgoing
    // track's `engineInfo` the verdict below is repainted and this test fails.
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    const sp = usePlayerStore.getState().signalPath;
    expect(sp?.sealLabel).not.toBe("BIT-PERFECT");
    expect(sp?.pure ?? false).toBe(false);
    // And nothing is displayable, so consumers render no seal at all.
    expect(sp?.active ?? false).toBe(false);
  });

  it("still pushes the new track to the engine (the clear is not a no-play)", async () => {
    await usePlayerStore.getState().playAt(1);
    expect(invoke).toHaveBeenCalled();
    expect(usePlayerStore.getState().currentIndex).toBe(1);
    expect(usePlayerStore.getState().engineActive).toBe(true);
  });
});

/**
 * The window between an input changing and Rust's verdict landing.
 *
 * Moving `derive` into `eko-core` made the seal ASYNCHRONOUS. `useSignalPath` is still a
 * synchronous store read, so between `setVolume(0.9)` and the IPC reply the store held the
 * PREVIOUS verdict — and React re-rendered on the volume change in between. A bit-perfect
 * 44.1 kHz FLAC dragged off unity therefore painted a green BIT-PERFECT, with the tooltip
 * "Untouched signal path — bit-for-bit to your DAC", over samples the engine had already
 * begun attenuating.
 *
 * The fix is asymmetric, and that asymmetry is the whole point: the frontend may WITHDRAW
 * a claim on its own (withdrawing asserts nothing) but may never MAKE one, because
 * resampling is an engine fact only Rust knows. So the store clears `pure` the moment an
 * input moves, and lets the pending reply restore the real verdict.
 *
 * Every assertion below is on the SYNCHRONOUS state — no `await` before the check — which
 * is exactly the render the user sees mid-drag.
 */
describe("the seal while a derivation is in flight", () => {
  /** A derivation that never answers, so the pending window stays open for inspection. */
  const pendingForever = () =>
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === "signal_path") return new Promise(() => {});
      if (cmd === "signal_replaygain") return { engineDb: null, sealDb: null };
      return null;
    });

  /** Restore the default mock for the suites that follow. */
  const respondNormally = () =>
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === "signal_path") return fakeDerive(args);
      if (cmd === "signal_replaygain") return { engineDb: null, sealDb: null };
      return null;
    });

  beforeEach(() => {
    invoke.mockClear();
    usePlayerStore.setState({
      tracks: [track("one", "/music/one.flac")],
      currentIndex: 0,
      engineActive: true,
      engineInfo: PREVIOUS_TRACK_INFO,
      volume: 1,
      eqEnabled: false,
      replayGainMode: "off",
      rgAppliedDb: null,
      // On screen right now: the strongest claim the product makes.
      signalPath: { ...BIT_PERFECT_SEAL },
    });
    pendingForever();
  });

  afterEach(respondNormally);

  it("stops claiming BIT-PERFECT in the same tick the volume leaves unity", () => {
    usePlayerStore.getState().setVolume(0.9);
    // No await: this is the render React performs for the `set({ volume })` above.
    const sp = usePlayerStore.getState().signalPath;
    expect(usePlayerStore.getState().volume).toBe(0.9);
    expect(sp?.pure ?? true).toBe(false);
    expect(sp?.sealLabel).not.toBe("BIT-PERFECT");
  });

  it("withdraws the long-form claim too, so no tooltip can read 'Bit-perfect'", () => {
    usePlayerStore.getState().setVolume(0.9);
    expect(usePlayerStore.getState().signalPath?.engineLabel).not.toBe("Bit-perfect");
  });

  it("names no modifier it has not been told about", () => {
    usePlayerStore.getState().setVolume(0.9);
    const sp = usePlayerStore.getState().signalPath;
    // Withdrawing a claim is honest; inventing "VOLUME" would be deriving the seal in the
    // frontend, which is the one thing this store must never do.
    expect(sp?.sealLabel).not.toBe("VOLUME");
    expect(sp?.attenuated ?? true).toBe(false);
  });

  it("does not blank or blink — the row and its nodes stay exactly as they were", () => {
    usePlayerStore.getState().setVolume(0.9);
    const sp = usePlayerStore.getState().signalPath;
    // `SignalPath.tsx` unmounts the whole row on `!active`; a drag must not do that.
    expect(sp).not.toBeNull();
    expect(sp?.active).toBe(true);
    expect(sp?.src).toBe(BIT_PERFECT_SEAL.src);
    expect(sp?.output).toBe(BIT_PERFECT_SEAL.output);
    expect(sp?.rgLabel).toBe(BIT_PERFECT_SEAL.rgLabel);
    // And the label stays displayable at both consumers — `Sidebar.tsx` renders
    // `${sealLabel} · 44.1 kHz`, which an empty string would turn into " · 44.1 kHz".
    expect(sp?.sealLabel).not.toBe("");
  });

  it("holds the withdrawal across the rest of a drag, without churning the label", () => {
    usePlayerStore.getState().setVolume(0.9);
    const first = usePlayerStore.getState().signalPath;
    // The 2nd..Nth ticks are swallowed by `syncVol`'s 50 ms throttle and never reach
    // `refreshSignalPath` at all.
    usePlayerStore.getState().setVolume(0.8);
    usePlayerStore.getState().setVolume(0.7);
    const last = usePlayerStore.getState().signalPath;
    expect(last?.pure ?? true).toBe(false);
    expect(last?.sealLabel).toBe(first?.sealLabel);
  });

  it("withdraws even on a tick the throttle swallows", () => {
    // Arms `syncVol`'s 50 ms throttle.
    usePlayerStore.getState().setVolume(0.9);
    // A verdict lands (or, as here, is put back) while the throttle is still closed.
    usePlayerStore.setState({ signalPath: { ...BIT_PERFECT_SEAL } });
    // This tick never reaches `refreshSignalPath` — it returns at the throttle gate — so
    // a withdrawal placed after that gate would leave BIT-PERFECT up for the full 50 ms.
    usePlayerStore.getState().setVolume(0.8);
    expect(usePlayerStore.getState().signalPath?.pure ?? true).toBe(false);
    expect(usePlayerStore.getState().signalPath?.sealLabel).not.toBe("BIT-PERFECT");
  });

  it("withdraws on an EQ change without evaluating the EQ itself", () => {
    usePlayerStore.getState().setEqEnabled(true);
    const sp = usePlayerStore.getState().signalPath;
    expect(sp?.pure ?? true).toBe(false);
    expect(sp?.sealLabel).not.toBe("BIT-PERFECT");
    expect(sp?.sealLabel).not.toBe("EQ");
  });

  it("withdraws on a ReplayGain mode change, which answers a whole round trip later", () => {
    usePlayerStore.getState().setReplayGainMode("album");
    const sp = usePlayerStore.getState().signalPath;
    expect(sp?.pure ?? true).toBe(false);
    expect(sp?.sealLabel).not.toBe("BIT-PERFECT");
  });

  it("lets Rust's reply win — the withdrawal is a pause, never a verdict", async () => {
    respondNormally();
    // Nothing else has moved, so the reply this fires is still valid when it lands.
    usePlayerStore.setState({ signalPath: { ...BIT_PERFECT_SEAL } });
    usePlayerStore.getState().setEqEnabled(false);
    expect(usePlayerStore.getState().signalPath?.pure).toBe(false);
    for (let i = 0; i < 5; i++) await Promise.resolve();
    expect(usePlayerStore.getState().signalPath?.sealLabel).toBe("BIT-PERFECT");
    expect(usePlayerStore.getState().signalPath?.pure).toBe(true);
  });

  /**
   * `sealGen` drops a reply a NEWER request has superseded — but `syncVol`'s throttle can
   * move the volume without sending one. Dragging down through unity could therefore land
   * a reply derived at volume 1.0 and repaint BIT-PERFECT over an attenuated stream.
   */
  it("rejects a reply derived at unity that lands after the volume moved", async () => {
    respondNormally();
    // `syncVol`'s throttle is module state and outlives a test. Let any timer an earlier
    // test armed lapse (50 ms, plus its trailing call's own 50 ms) — otherwise the first
    // `setVolume` below is swallowed, no derivation is ever sent, and this test passes
    // without exercising anything.
    await new Promise((r) => setTimeout(r, 200));
    usePlayerStore.setState({ signalPath: { ...BIT_PERFECT_SEAL }, volume: 1 });
    // Dispatches a derivation for volume 1.0 and arms the throttle.
    usePlayerStore.getState().setVolume(1);
    // Inside the 50 ms window: swallowed, so no newer request bumps `sealGen` and the
    // unity reply is still the newest when it lands.
    usePlayerStore.getState().setVolume(0.9);
    for (let i = 0; i < 5; i++) await Promise.resolve();
    const sp = usePlayerStore.getState().signalPath;
    expect(usePlayerStore.getState().volume).toBe(0.9);
    expect(sp?.pure ?? true).toBe(false);
    expect(sp?.sealLabel).not.toBe("BIT-PERFECT");
  });
});

/**
 * ReplayGain is decided in Rust (`eko_core::signal_path::replaygain_decision`) — both the
 * peak-limited dB for the engine and the dead-banded dB for the seal. The ±0.01 dB
 * dead-band is the boundary that settles whether EKO claims bit-perfect, so the store must
 * forward the track's tags and consume the answer rather than compute either number.
 */
describe("ReplayGain is decided in Rust, not in the store", () => {
  const tagged = (): Track => ({
    ...track("rg", "/music/rg.flac"),
    rgTrackGain: -6.5,
    rgTrackPeak: 0.9,
    rgAlbumGain: -4.25,
    rgAlbumPeak: 0.95,
  });

  beforeEach(() => {
    invoke.mockClear();
    usePlayerStore.setState({
      tracks: [tagged()],
      currentIndex: 0,
      replayGainMode: "album",
      rgAppliedDb: null,
    });
  });

  it("forwards the track's four tags and the mode, and derives nothing itself", async () => {
    usePlayerStore.getState().setReplayGainMode("album");
    await Promise.resolve();
    const call = invoke.mock.calls.find(([cmd]) => cmd === "signal_replaygain");
    expect(call, "the store must ask Rust for the ReplayGain decision").toBeDefined();
    expect(call?.[1]).toEqual({
      tags: { trackGain: -6.5, trackPeak: 0.9, albumGain: -4.25, albumPeak: 0.95 },
      mode: "album",
    });
  });

  it("reports the seal dB Rust returned, not the engine dB", async () => {
    // A negligible correction: real enough for the engine, inaudible for the seal.
    invoke.mockImplementation(async (cmd: string) =>
      cmd === "signal_replaygain" ? { engineDb: 0.004, sealDb: null } : null,
    );
    usePlayerStore.getState().setReplayGainMode("track");
    await Promise.resolve();
    await Promise.resolve();
    expect(usePlayerStore.getState().rgAppliedDb).toBeNull();

    // And a real one is reported.
    invoke.mockImplementation(async (cmd: string) =>
      cmd === "signal_replaygain" ? { engineDb: -6.5, sealDb: -6.5 } : null,
    );
    usePlayerStore.getState().setReplayGainMode("album");
    await Promise.resolve();
    await Promise.resolve();
    expect(usePlayerStore.getState().rgAppliedDb).toBe(-6.5);
  });

  it("sends the engine the un-dead-banded value Rust chose", async () => {
    invoke.mockImplementation(async (cmd: string) =>
      cmd === "signal_replaygain" ? { engineDb: 0.004, sealDb: null } : null,
    );
    usePlayerStore.getState().setReplayGainMode("track");
    await Promise.resolve();
    await Promise.resolve();
    const call = invoke.mock.calls.find(([cmd]) => cmd === "engine_set_replaygain");
    expect(call?.[1]).toEqual({ gainDb: 0.004 });
  });

  /**
   * The failure path must not assert. "No ReplayGain" is itself a claim — and the
   * strongest one the product makes — so if a decision fails after an earlier one already
   * pushed a real gain to the engine, reporting null would leave the seal saying
   * BIT-PERFECT over gain-adjusted audio.
   */
  describe("when the decision fails", () => {
    const failRg = () =>
      invoke.mockImplementation(async (cmd: string, args?: unknown) => {
        if (cmd === "signal_replaygain") throw new Error("ipc down");
        if (cmd === "signal_path") return fakeDerive(args);
        return null;
      });

    it("drops the seal instead of asserting anything about the signal", async () => {
      usePlayerStore.setState({ rgAppliedDb: -6.5, signalPath: { ...BIT_PERFECT_SEAL } });
      failRg();
      usePlayerStore.getState().setReplayGainMode("track");
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      expect(usePlayerStore.getState().signalPath).toBeNull();
      expect(usePlayerStore.getState().rgAppliedDb).toBeNull();
    });

    it("clears the gain in the engine so it matches what we can honestly claim", async () => {
      // A gain is already applied in the engine from an earlier successful decision.
      usePlayerStore.setState({ rgAppliedDb: -6.5 });
      failRg();
      invoke.mockClear();
      usePlayerStore.getState().setReplayGainMode("track");
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      const call = invoke.mock.calls.find(([cmd]) => cmd === "engine_set_replaygain");
      expect(call, "the engine must be told to stop applying the old gain").toBeDefined();
      expect(call?.[1]).toEqual({ gainDb: null });
    });

    it("does not leave a derived seal behind for a later refresh to resurrect", async () => {
      usePlayerStore.setState({ signalPath: { ...BIT_PERFECT_SEAL } });
      failRg();
      usePlayerStore.getState().setReplayGainMode("track");
      for (let i = 0; i < 5; i++) await Promise.resolve();
      expect(usePlayerStore.getState().signalPath).toBeNull();
    });
  });
});
