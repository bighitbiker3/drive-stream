// Render review stills: node scripts/stills.mjs 40 150 254 …
import {bundle} from "@remotion/bundler";
import {renderStill, selectComposition} from "@remotion/renderer";
import path from "node:path";

const frames = process.argv.slice(2).map(Number);
const browserExecutable = process.env.BROWSER || undefined;
const serveUrl = await bundle({entryPoint: path.resolve("src/index.ts")});
const composition = await selectComposition({serveUrl, id: "DriveStreamPromo", browserExecutable});
for (const frame of frames) {
  const output = `out/still-${String(frame).padStart(4, "0")}.png`;
  await renderStill({serveUrl, composition, frame, output, browserExecutable, scale: 0.5});
  console.log(output);
}
