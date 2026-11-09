import React from "react";
import {AbsoluteFill, spring, useCurrentFrame, useVideoConfig} from "remotion";
import {Background, GradientText, Rise, useOut} from "../components";
import {C, mono} from "../theme";
import {dur} from "../timeline";

export const CREATIVE_DURATION = dur("creative");

const PANEL_W = 830;
const PANEL_H = 600;
const PANEL_TOP = 215;

/** Deterministic pseudo-random numbers, so every render is identical. */
const rand = (seed: number) => {
  const x = Math.sin(seed * 12.9898) * 43758.5453;
  return x - Math.floor(x);
};

const Chip: React.FC<{children: React.ReactNode}> = ({children}) => (
  <div
    style={{
      position: "absolute",
      left: 22,
      top: 20,
      zIndex: 2,
      padding: "6px 14px",
      borderRadius: 99,
      fontSize: 18,
      fontWeight: 800,
      letterSpacing: 0.4,
      background: "rgba(5,7,13,0.75)",
      border: `1px solid ${C.line}`,
    }}
  >
    {children}
  </div>
);

const Panel: React.FC<{left: number; delay: number; children: React.ReactNode}> = ({left, delay, children}) => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const p = spring({frame: f - delay, fps, config: {damping: 200, mass: 0.8}});
  return (
    <div
      style={{
        position: "absolute",
        left,
        top: PANEL_TOP,
        width: PANEL_W,
        height: PANEL_H,
        borderRadius: 22,
        overflow: "hidden",
        background: "rgba(22,24,32,0.92)",
        border: `1px solid ${C.line}`,
        boxShadow: "0 40px 110px rgba(0,0,0,0.55)",
        opacity: p,
        transform: `translateY(${(1 - p) * 60}px)`,
      }}
    >
      {children}
    </div>
  );
};

/* ---------- left: an editing timeline being scrubbed ---------- */

const tracks = [
  {label: "V2", clips: [[0.08, 0.33, "#8b5cf6"], [0.52, 0.78, "#8b5cf6"]]},
  {label: "V1", clips: [[0.0, 0.29, "#0ea5e9"], [0.29, 0.61, "#f97316"], [0.61, 1.0, "#14b8a6"]]},
] as const;

const Editor: React.FC<{f: number}> = ({f}) => {
  // scrub back and forth, with a couple of quick jumps like a real edit
  const base = 0.5 + 0.38 * Math.sin(f / 24) + 0.06 * Math.sin(f / 7);
  const jump = f > 150 && f < 200 ? 0.22 : 0;
  const ph = Math.min(0.97, Math.max(0.03, base - jump));
  const tlLeft = 80;
  const tlW = PANEL_W - tlLeft - 24;
  const hue = Math.round(ph * 300);
  const seconds = Math.round(ph * 1460);
  const tc = `00:${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}:${String(f % 25).padStart(2, "0")}`;
  return (
    <>
      <Chip>VIDEO EDITING</Chip>
      {/* viewer */}
      <div
        style={{
          position: "absolute",
          left: 24,
          right: 24,
          top: 24,
          height: 300,
          borderRadius: 12,
          overflow: "hidden",
          background: `linear-gradient(135deg, hsl(${hue},70%,55%), hsl(${(hue + 60) % 360},65%,30%))`,
        }}
      >
        <div style={{position: "absolute", left: "12%", top: "58%", width: "76%", height: "30%", borderRadius: "50% 50% 0 0", background: "rgba(0,0,0,0.25)"}} />
        <div style={{position: "absolute", right: 16, bottom: 12, fontFamily: mono, fontSize: 20, color: "#fff", textShadow: "0 1px 4px rgba(0,0,0,0.6)"}}>{tc}</div>
        <div style={{position: "absolute", left: 16, bottom: 12, fontSize: 16, fontWeight: 700, color: "#fff", opacity: 0.85}}>4K · ProRes 422 HQ</div>
      </div>
      {/* timeline */}
      <div style={{position: "absolute", left: 24, right: 24, top: 348, bottom: 24, borderRadius: 12, background: "rgba(0,0,0,0.3)"}}>
        {[...tracks, {label: "A1", clips: [] as unknown as typeof tracks[0]["clips"]}].map((t, row) => (
          <div key={t.label} style={{position: "absolute", left: 0, right: 0, top: 18 + row * 72, height: 60}}>
            <div style={{position: "absolute", left: 14, top: 18, fontSize: 18, fontWeight: 800, color: C.muted}}>{t.label}</div>
            {t.label === "A1" ? (
              <div style={{position: "absolute", left: tlLeft - 24, width: tlW, top: 4, height: 52, display: "flex", alignItems: "center", gap: 2, borderRadius: 8, background: "rgba(63,209,122,0.16)", padding: "0 6px"}}>
                {Array.from({length: 120}, (_, i) => (
                  <div key={i} style={{flex: 1, height: `${18 + 70 * Math.abs(Math.sin(i * 0.37) * rand(i))}%`, background: C.green, opacity: 0.7, borderRadius: 1}} />
                ))}
              </div>
            ) : (
              t.clips.map(([a, b, color], k) => {
                const under = ph >= a && ph < b;
                return (
                  <div
                    key={k}
                    style={{
                      position: "absolute",
                      left: tlLeft - 24 + a * tlW + 2,
                      width: (b - a) * tlW - 4,
                      top: 4,
                      height: 52,
                      borderRadius: 8,
                      background: `repeating-linear-gradient(90deg, ${color} 0 54px, rgba(0,0,0,0.25) 54px 56px)`,
                      opacity: 0.85,
                      overflow: "hidden",
                    }}
                  >
                    {under && (
                      <div
                        style={{
                          position: "absolute",
                          left: (ph - a) * tlW - 26,
                          width: 52,
                          top: 0,
                          bottom: 0,
                          background: `linear-gradient(90deg, transparent, ${C.cyan}, transparent)`,
                          boxShadow: `0 0 24px ${C.cyan}`,
                        }}
                      />
                    )}
                  </div>
                );
              })
            )}
          </div>
        ))}
        <div style={{position: "absolute", left: tlLeft - 24 + ph * tlW - 1, top: 6, bottom: 6, width: 3, background: "#ff3b5c", boxShadow: "0 0 12px #ff3b5c"}} />
      </div>
    </>
  );
};

/* ---------- right: a sample browser previewing a library in Drive ---------- */

const samples = [
  ["Kick_Punchy_01.wav", "0:01"],
  ["Kick_Sub_Tail_04.wav", "0:02"],
  ["Snare_Room_03.wav", "0:01"],
  ["808_Long_C.wav", "0:03"],
  ["Vox_Chop_Am_120.wav", "0:04"],
  ["Pad_Warm_Dm.wav", "0:08"],
  ["Riser_Big_8bar.wav", "0:16"],
  ["Hat_Open_Dusty.wav", "0:01"],
  ["Perc_Shaker_Loop.wav", "0:04"],
];
const STEP = 24;
const FIRST = 34;

const Browser: React.FC<{f: number}> = ({f}) => {
  const current = Math.floor((f - FIRST) / STEP);
  const within = (f - FIRST) % STEP;
  const cachedMB = 46 + Math.max(0, Math.min(samples.length, current + 1)) * 4.6;
  return (
    <>
      <Chip>SAMPLE LIBRARIES</Chip>
      <div style={{position: "absolute", left: 0, top: 0, bottom: 0, width: 220, background: "rgba(0,0,0,0.28)", padding: "78px 0 0"}}>
        <div style={{fontSize: 15, fontWeight: 800, color: C.dim, letterSpacing: 1, padding: "0 22px 10px"}}>PLACES</div>
        {["Drive Stream", "Samples", "Drums", "808s", "Vocals", "FX", "Loops"].map((p, i) => (
          <div
            key={p}
            style={{
              padding: "9px 22px",
              fontSize: 19,
              fontWeight: i === 0 || i === 2 ? 700 : 500,
              color: i === 2 ? "#fff" : C.muted,
              background: i === 2 ? "rgba(58,123,255,0.35)" : "transparent",
              paddingLeft: i === 0 ? 22 : 40,
            }}
          >
            {p}
          </div>
        ))}
      </div>
      <div style={{position: "absolute", left: 220, right: 0, top: 0, bottom: 0, padding: "22px 22px"}}>
        <div style={{display: "flex", justifyContent: "flex-end", gap: 10, fontSize: 16, fontFamily: mono}}>
          <span style={{padding: "5px 11px", borderRadius: 8, background: "rgba(255,255,255,0.07)", color: C.muted}}>84 GB in Drive</span>
          <span style={{padding: "5px 11px", borderRadius: 8, background: "rgba(54,209,255,0.14)", color: C.cyan}}>{cachedMB.toFixed(0)} MB on this Mac</span>
        </div>
        <div style={{fontSize: 17, color: C.dim, margin: "18px 4px 10px"}}>Drive Stream › Samples › Drums</div>
        {samples.map(([name, len], i) => {
          const playing = i === current && f >= FIRST;
          const heard = i < current;
          return (
            <div
              key={name}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 14,
                height: 50,
                padding: "0 12px",
                borderRadius: 9,
                background: playing ? "rgba(58,123,255,0.28)" : "transparent",
              }}
            >
              <div style={{width: 18, fontSize: 16, color: playing ? "#fff" : C.dim}}>{playing ? "▶" : ""}</div>
              <div style={{width: 238, fontSize: 18, fontWeight: playing ? 700 : 500, color: playing ? "#fff" : C.text, whiteSpace: "nowrap"}}>{name}</div>
              <div style={{flex: 1, height: 30, display: "flex", alignItems: "center", gap: 2, position: "relative"}}>
                {Array.from({length: 46}, (_, k) => {
                  const h = 15 + 85 * Math.abs(Math.sin((k + 1) * (0.5 + i * 0.13))) * Math.exp(-k / (8 + i * 4)) + 8 * rand(i * 100 + k);
                  const lit = playing && k / 46 < within / STEP;
                  return <div key={k} style={{flex: 1, height: `${Math.min(100, h)}%`, borderRadius: 1, background: lit ? C.cyan : heard ? "rgba(54,209,255,0.45)" : "rgba(255,255,255,0.22)"}} />;
                })}
              </div>
              <div style={{width: 58, textAlign: "right", fontSize: 15, fontFamily: mono, color: C.dim}}>
                {playing && within > 2 ? <span style={{color: C.green}}>0.4 s</span> : len}
              </div>
            </div>
          );
        })}
      </div>
    </>
  );
};

export const Creative: React.FC = () => {
  const f = useCurrentFrame();
  const out = useOut(CREATIVE_DURATION);
  return (
    <Background glow={1}>
      <AbsoluteFill style={{opacity: out}}>
        <Rise delay={2} style={{position: "absolute", top: 82, width: "100%", textAlign: "center"}}>
          <div style={{fontSize: 70, fontWeight: 900, letterSpacing: -2}}>
            Made for <GradientText>creative work.</GradientText>
          </div>
        </Rise>
        <Panel left={110} delay={10}>
          <Editor f={f} />
        </Panel>
        <Panel left={980} delay={20}>
          <Browser f={f} />
        </Panel>
        <Rise delay={30} style={{position: "absolute", left: 110, top: PANEL_TOP + PANEL_H + 34, width: PANEL_W}}>
          <div style={{fontSize: 36, fontWeight: 800, letterSpacing: -0.5}}>Cut 4K straight from Drive</div>
          <div style={{fontSize: 23, color: C.muted, marginTop: 8, lineHeight: 1.4}}>
            DaVinci Resolve, Premiere Pro and friends. Scrub anywhere; only what you touch downloads.
          </div>
        </Rise>
        <Rise delay={40} style={{position: "absolute", left: 980, top: PANEL_TOP + PANEL_H + 34, width: PANEL_W}}>
          <div style={{fontSize: 36, fontWeight: 800, letterSpacing: -0.5}}>Browse sample libraries in Ableton Live</div>
          <div style={{fontSize: 23, color: C.muted, marginTop: 8, lineHeight: 1.4}}>
            Previews start in under a second. The whole library stays in the cloud.
          </div>
        </Rise>
      </AbsoluteFill>
    </Background>
  );
};
