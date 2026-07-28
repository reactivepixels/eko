/**
 * Free visualizer registry — Galaxy only.
 *
 * Cymatics and Murmuration are Pro-only and registered separately in
 * src/pro/visualizers/registry.ts, which imports Galaxy from here too.
 */

import type { VisualizerDef } from "./types";
import { galaxy } from "./galaxy";

export const visualizers: VisualizerDef[] = [galaxy];
