import {loadFont} from "@remotion/google-fonts/Inter";

export const {fontFamily} = loadFont("normal", {weights: ["400", "500", "600", "700", "800", "900"], subsets: ["latin"]});
export const mono = "ui-monospace, SFMono-Regular, Menlo, monospace";

export const C = {
  bg: "#05070d",
  text: "#f4f6ff",
  muted: "#9aa3b8",
  dim: "#5d667c",
  cyan: "#36D1FF",
  blue: "#3A7BFF",
  indigo: "#4B3BEA",
  green: "#3fd17a",
  red: "#ff5c6c",
  amber: "#ffb340",
  card: "rgba(255,255,255,0.055)",
  line: "rgba(255,255,255,0.11)",
};

export const brand = `linear-gradient(120deg, ${C.cyan} 0%, ${C.blue} 55%, ${C.indigo} 100%)`;
export const FPS = 30;
