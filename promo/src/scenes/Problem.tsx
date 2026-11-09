import React from "react";
import {AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig} from "remotion";
import {Background, ease, Rise, useOut} from "../components";
import {C, mono} from "../theme";
import {dur} from "../timeline";

export const PROBLEM_DURATION = dur("problem");

const rows = [
  {name: "Weekend trip 4K.mov", size: "4.2 GB"},
  {name: "Drone pass.mov", size: "1.8 GB"},
  {name: "Interview A-cam.mov", size: "6.1 GB"},
  {name: "B-roll selects.mov", size: "2.7 GB"},
];

const CloudIcon: React.FC = () => (
  <svg width="22" height="22" viewBox="0 0 24 24">
    <path d="M7 18h10.5a4 4 0 0 0 .5-7.97A5.5 5.5 0 0 0 7.4 11 3.5 3.5 0 0 0 7 18z" fill="none" stroke={C.muted} strokeWidth="1.6" />
  </svg>
);

const Cursor: React.FC<{x: number; y: number; press: number}> = ({x, y, press}) => (
  <div style={{position: "absolute", left: x, top: y, transform: `scale(${1 - press * 0.15})`, transformOrigin: "top left"}}>
    <svg width="34" height="34" viewBox="0 0 24 24">
      <path d="M5 3l14 8.5-6.2 1.3L9.6 19z" fill="#fff" stroke="#000" strokeWidth="1.2" strokeLinejoin="round" />
    </svg>
  </div>
);

export const Problem: React.FC = () => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const out = useOut(PROBLEM_DURATION);
  const win = spring({frame: f - 4, fps, config: {damping: 200}});

  // cursor travels to the first row and double-clicks
  const cx = interpolate(f, [10, 42], [1250, 660], {extrapolateLeft: "clamp", extrapolateRight: "clamp"});
  const cy = interpolate(f, [10, 42], [760, 372], {extrapolateLeft: "clamp", extrapolateRight: "clamp"});
  const press = f >= 46 && f < 50 ? 1 : f >= 53 && f < 57 ? 1 : 0;
  const selected = f >= 46;

  // Google Drive's "download first" sheet
  const sheet = spring({frame: f - 60, fps, config: {damping: 18, mass: 0.7}});
  const t = Math.max(0, f - 66) / 30; // seconds since the download started
  const got = Math.min(4.2e9, 1.6e6 * t + 6e6 * Math.min(1, t)); // ~1.6 MB/s
  const pct = got / 4.2e9;
  const minsLeft = Math.ceil((4.2e9 - got) / 1.6e6 / 60);
  const spin = f * 9;

  return (
    <Background glow={0.55}>
      <AbsoluteFill style={{opacity: out}}>
        <Rise delay={70} style={{position: "absolute", top: 86, width: "100%", textAlign: "center"}}>
          <div style={{fontSize: 58, fontWeight: 800, letterSpacing: -1.5}}>
            Google Drive makes you <span style={{color: C.red}}>download the whole file</span> first.
          </div>
        </Rise>

        {/* Finder window */}
        <div
          style={{
            position: "absolute",
            left: 410,
            top: 240,
            width: 1100,
            height: 440,
            borderRadius: 18,
            background: "rgba(28,30,38,0.92)",
            border: `1px solid ${C.line}`,
            boxShadow: "0 40px 120px rgba(0,0,0,0.55)",
            opacity: win,
            transform: `translateY(${(1 - win) * 40}px) scale(${0.96 + win * 0.04})`,
            overflow: "hidden",
          }}
        >
          <div style={{height: 56, display: "flex", alignItems: "center", gap: 10, padding: "0 20px", borderBottom: `1px solid ${C.line}`}}>
            {["#ff5f57", "#febc2e", "#28c840"].map((c) => (
              <div key={c} style={{width: 14, height: 14, borderRadius: 7, background: c}} />
            ))}
            <div style={{marginLeft: 22, fontSize: 20, fontWeight: 600, color: C.muted}}>Google Drive › My Drive › Videos</div>
          </div>
          <div style={{padding: "14px 22px"}}>
            <div style={{display: "flex", fontSize: 16, color: C.dim, padding: "6px 14px", fontWeight: 600}}>
              <div style={{flex: 1}}>Name</div>
              <div style={{width: 140, textAlign: "right"}}>Size</div>
              <div style={{width: 60}} />
            </div>
            {rows.map((r, i) => (
              <div
                key={r.name}
                style={{
                  display: "flex",
                  alignItems: "center",
                  fontSize: 24,
                  padding: "16px 14px",
                  borderRadius: 10,
                  background: i === 0 && selected ? "rgba(58,123,255,0.85)" : "transparent",
                }}
              >
                <div style={{width: 34, height: 26, borderRadius: 5, background: "linear-gradient(135deg,#6b7280,#374151)", marginRight: 16}} />
                <div style={{flex: 1, fontWeight: 500}}>{r.name}</div>
                <div style={{width: 140, textAlign: "right", color: i === 0 && selected ? "#fff" : C.muted}}>{r.size}</div>
                <div style={{width: 60, display: "flex", justifyContent: "flex-end"}}>
                  <CloudIcon />
                </div>
              </div>
            ))}
          </div>

          {/* download sheet */}
          <div
            style={{
              position: "absolute",
              left: 110,
              right: 110,
              top: 104,
              padding: "30px 36px",
              borderRadius: 16,
              background: "rgba(44,46,56,0.98)",
              border: `1px solid ${C.line}`,
              boxShadow: "0 30px 80px rgba(0,0,0,0.6)",
              opacity: sheet,
              transform: `translateY(${(1 - sheet) * -30}px)`,
            }}
          >
            <div style={{display: "flex", alignItems: "center", gap: 18}}>
              <svg width="34" height="34" viewBox="0 0 34 34" style={{transform: `rotate(${spin}deg)`}}>
                <circle cx="17" cy="17" r="13" stroke="rgba(255,255,255,0.15)" strokeWidth="4" fill="none" />
                <path d="M17 4a13 13 0 0 1 13 13" stroke={C.muted} strokeWidth="4" fill="none" strokeLinecap="round" />
              </svg>
              <div style={{fontSize: 27, fontWeight: 700}}>Downloading “Weekend trip 4K.mov”…</div>
            </div>
            <div style={{height: 10, borderRadius: 5, background: "rgba(255,255,255,0.1)", margin: "24px 0 14px", overflow: "hidden"}}>
              <div style={{width: `${Math.max(1.2, pct * 100)}%`, height: "100%", background: C.muted, borderRadius: 5}} />
            </div>
            <div style={{display: "flex", justifyContent: "space-between", fontSize: 22, fontFamily: mono}}>
              <span style={{color: C.muted}}>{Math.round(got / 1e6)} MB of 4.2 GB · 1.6 MB/s</span>
              <span style={{color: C.amber}}>about {minsLeft} minutes remaining</span>
            </div>
          </div>
        </div>

        {f < 64 && <Cursor x={cx} y={cy} press={press} />}

        <Rise delay={128} style={{position: "absolute", bottom: 150, width: "100%", textAlign: "center"}}>
          <div style={{fontSize: 34, color: C.muted, fontWeight: 500}}>
            4.2 GB of waiting. <span style={{color: C.text}}>4.2 GB of your disk.</span>
          </div>
        </Rise>
      </AbsoluteFill>
      <AbsoluteFill style={{background: "#000", opacity: ease(f, 0, 10, 1, 0)}} />
    </Background>
  );
};
