import React from "react";
import {AbsoluteFill, interpolate, useCurrentFrame} from "remotion";
import {Background, ease, GradientText, useOut} from "../components";
import {C, mono} from "../theme";
import {dur} from "../timeline";

export const HOW_DURATION = dur("how");

const N = 60; // blocks drawn for the 4.2 GB file
const PITCH = 26;
const LEFT = (1920 - N * PITCH + 4) / 2;
const SEEK = 132;

/** Playhead position in blocks. */
const position = (f: number) =>
  f < SEEK
    ? interpolate(f, [20, 125], [6, 13], {extrapolateLeft: "clamp", extrapolateRight: "clamp"})
    : interpolate(f, [SEEK, SEEK + 6, 265], [13, 40, 46], {extrapolateRight: "clamp"});

/** When each block's download starts (Infinity: never in this scene). */
const enterFrame = (i: number) => {
  if (i >= 6 && i <= 12) return 20 + (i - 6) * 2.5; // initial read-ahead, fetched in parallel
  if (i > 12 && i <= 19) return 20 + ((i - 12) / 7) * 105; // read-ahead keeps pace with playback
  if (i >= 40 && i <= 46) return SEEK + 4 + (i - 40) * 1.2; // after a seek: the new spot first
  if (i > 46 && i <= 52) return SEEK + 8 + ((i - 46) / 6) * 125;
  return Infinity;
};

const Captions: React.FC<{f: number}> = ({f}) => {
  const items: [number, number, React.ReactNode][] = [
    [0, 128, <>Only the bytes you play are <GradientText>fetched.</GradientText></>],
    [128, 205, <>Jump anywhere. Playing again in <GradientText>half a second.</GradientText></>],
    [205, HOW_DURATION, <>Read-ahead keeps 4K smooth. <GradientText>The rest stays in the cloud.</GradientText></>],
  ];
  return (
    <div style={{position: "absolute", top: 70, width: "100%", height: 80, textAlign: "center"}}>
      {items.map(([a, b, text], k) => {
        const o = Math.min(ease(f, a, a + 12), 1 - ease(f, b - 10, b));
        return (
          <div
            key={k}
            style={{
              position: "absolute",
              width: "100%",
              fontSize: 58,
              fontWeight: 800,
              letterSpacing: -1.5,
              opacity: k === 0 ? Math.min(1 - ease(f, b - 10, b), ease(f, 4, 16)) : o,
              transform: `translateY(${(1 - ease(f, a, a + 14)) * 24}px)`,
            }}
          >
            {text}
          </div>
        );
      })}
    </div>
  );
};

/** A stand-in "video": a sunset for the first part, the sea after the seek. */
const Player: React.FC<{f: number; pos: number}> = ({f, pos}) => {
  const atSea = pos > 30;
  const flash = f >= SEEK && f < SEEK + 10 ? 1 - (f - SEEK) / 10 : 0;
  const drift = (pos % 10) * 18;
  const seconds = Math.round((pos / N) * 370);
  const tc = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
  return (
    <div
      style={{
        position: "absolute",
        left: 480,
        top: 175,
        width: 960,
        height: 400,
        borderRadius: 20,
        overflow: "hidden",
        border: `1px solid ${C.line}`,
        boxShadow: "0 30px 90px rgba(0,0,0,0.55)",
      }}
    >
      <svg width="960" height="400" viewBox="0 0 960 400" style={{position: "absolute", inset: 0}}>
        <defs>
          <linearGradient id="sky1" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#ff8a5c" />
            <stop offset="0.55" stopColor="#ffbd6e" />
            <stop offset="1" stopColor="#6b3a7a" />
          </linearGradient>
          <linearGradient id="sky2" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#0f3b66" />
            <stop offset="0.6" stopColor="#1e7fb3" />
            <stop offset="1" stopColor="#0a2a4a" />
          </linearGradient>
        </defs>
        <rect width="960" height="400" fill={atSea ? "url(#sky2)" : "url(#sky1)"} />
        {atSea ? (
          <>
            <circle cx={700 - drift * 0.3} cy={110} r={46} fill="#e9f6ff" opacity={0.9} />
            {[0, 1, 2, 3].map((k) => (
              <path
                key={k}
                d={`M ${-200 + ((drift * (k + 1)) % 200)} ${250 + k * 34} q 60 -18 120 0 t 120 0 t 120 0 t 120 0 t 120 0 t 120 0 t 120 0 t 120 0 t 120 0 t 120 0`}
                stroke="#bfe6ff"
                strokeOpacity={0.35 - k * 0.06}
                strokeWidth={3}
                fill="none"
              />
            ))}
          </>
        ) : (
          <>
            <circle cx={480} cy={230 + drift * 0.15} r={70} fill="#fff3c4" opacity={0.95} />
            <path d={`M 0 300 L ${160 - drift} 200 L ${330 - drift} 290 L ${520 - drift} 170 L ${760 - drift} 300 L 960 230 L 960 400 L 0 400 Z`} fill="#4a2a5c" />
            <path d={`M 0 340 L ${240 - drift * 1.6} 260 L ${480 - drift * 1.6} 330 L ${700 - drift * 1.6} 250 L 960 330 L 960 400 L 0 400 Z`} fill="#2c1838" />
          </>
        )}
      </svg>
      <div style={{position: "absolute", inset: 0, background: "#fff", opacity: flash * 0.5}} />
      <div
        style={{
          position: "absolute",
          left: 0,
          right: 0,
          bottom: 0,
          height: 64,
          display: "flex",
          alignItems: "center",
          gap: 18,
          padding: "0 24px",
          background: "linear-gradient(transparent, rgba(0,0,0,0.65))",
        }}
      >
        <svg width="22" height="22" viewBox="0 0 24 24">
          <rect x="5" y="4" width="5" height="16" rx="1.5" fill="#fff" />
          <rect x="14" y="4" width="5" height="16" rx="1.5" fill="#fff" />
        </svg>
        <div style={{flex: 1, height: 6, borderRadius: 3, background: "rgba(255,255,255,0.3)"}}>
          <div style={{width: `${(pos / N) * 100}%`, height: "100%", borderRadius: 3, background: "#fff"}} />
        </div>
        <div style={{fontFamily: mono, fontSize: 20, color: "#fff"}}>{tc} / 6:10</div>
      </div>
    </div>
  );
};

export const HowItWorks: React.FC = () => {
  const f = useCurrentFrame();
  const out = useOut(HOW_DURATION);
  const pos = position(f);
  const stripTop = 650;
  const fetched = Array.from({length: N}, (_, i) => Math.max(0, Math.min(1, (f - enterFrame(i)) / 10)));
  const fetchedShare = fetched.reduce((a, b) => a + b, 0) / N;

  return (
    <Background glow={0.9}>
      <AbsoluteFill style={{opacity: out}}>
        <Captions f={f} />
        <Player f={f} pos={pos} />

        <div style={{position: "absolute", left: LEFT, top: stripTop - 70, fontSize: 22, color: C.muted, fontWeight: 600}}>
          Weekend trip 4K.mov <span style={{color: C.dim}}>· 4.2 GB in Google Drive</span>
        </div>
        {fetched.map((v, i) => {
          const arriving = v > 0 && v < 1;
          const cached = f >= SEEK + 10 && i < 30 && v === 1;
          return (
            <div
              key={i}
              style={{
                position: "absolute",
                left: LEFT + i * PITCH,
                top: stripTop,
                width: PITCH - 4,
                height: 96,
                borderRadius: 6,
                background: "rgba(255,255,255,0.07)",
                overflow: "hidden",
              }}
            >
              <div
                style={{
                  position: "absolute",
                  left: 0,
                  right: 0,
                  bottom: 0,
                  height: `${v * 100}%`,
                  background: arriving
                    ? `linear-gradient(180deg, ${C.cyan}, ${C.blue})`
                    : `linear-gradient(180deg, ${C.blue}, ${C.indigo})`,
                  opacity: cached ? 0.5 : 1,
                  boxShadow: arriving ? `0 0 24px ${C.cyan}` : "none",
                }}
              />
            </div>
          );
        })}
        {/* playhead */}
        <div
          style={{
            position: "absolute",
            left: LEFT + pos * PITCH - 2,
            top: stripTop - 14,
            width: 4,
            height: 124,
            borderRadius: 2,
            background: "#fff",
            boxShadow: "0 0 18px rgba(255,255,255,0.8)",
          }}
        />
        <div style={{position: "absolute", left: LEFT + pos * PITCH - 9, top: stripTop - 24, width: 18, height: 18, borderRadius: 9, background: "#fff"}} />

        <div style={{position: "absolute", top: stripTop + 132, width: "100%", display: "flex", justifyContent: "center", gap: 70, fontSize: 26}}>
          <div>
            <span style={{color: C.muted}}>Fetched from Drive </span>
            <span style={{fontWeight: 800, fontFamily: mono}}>{Math.round(fetchedShare * 100)}%</span>
          </div>
          <div>
            <span style={{color: C.muted}}>Waiting for the rest </span>
            <span style={{fontWeight: 800, color: C.green}}>never</span>
          </div>
          <div style={{display: "flex", alignItems: "center", gap: 10}}>
            <div style={{width: 18, height: 18, borderRadius: 4, background: `linear-gradient(${C.cyan}, ${C.blue})`}} />
            <span style={{color: C.muted}}>streaming</span>
            <div style={{width: 18, height: 18, borderRadius: 4, marginLeft: 16, background: C.blue, opacity: 0.5}} />
            <span style={{color: C.muted}}>cached on your Mac</span>
          </div>
        </div>
      </AbsoluteFill>
    </Background>
  );
};
