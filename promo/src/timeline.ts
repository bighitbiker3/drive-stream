import timeline from "./timeline.json";

type Scene = "problem" | "reveal" | "how" | "speed" | "creative" | "uploads" | "dashboard" | "features" | "outro";

/** Scene lengths and transitions, shared with scripts/make-music.py. */
export const TRANSITION = timeline.transition;
export const SCENES = timeline.scenes as [Scene, number][];
export const dur = (s: Scene) => SCENES.find(([n]) => n === s)![1];
export const TOTAL = SCENES.reduce((a, [, d]) => a + d, 0) - TRANSITION * (SCENES.length - 1);
