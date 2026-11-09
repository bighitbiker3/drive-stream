import React from "react";
import {AbsoluteFill, Img, interpolate, spring, staticFile, useCurrentFrame, useVideoConfig} from "remotion";
import {Background, Check, ease, fmtBytes, GradientText, Rise, useOut} from "../components";
import {brand, C} from "../theme";
import {dur} from "../timeline";

export const UPLOADS_DURATION = dur("uploads");

const files = [
  {name: "VID_20261108_101530.mp4", size: 640e6, start: 38, rate: 8.2e6},
  {name: "Drone pass.mov", size: 380e6, start: 44, rate: 4.2e6},
  {name: "Interview A-cam.mov", size: 1.9e9, start: 50, rate: 5.6e6},
];

const progress = (f: number, x: (typeof files)[number]) => Math.max(0, Math.min(1, ((f - x.start) * x.rate) / x.size));
const doneAt = (x: (typeof files)[number]) => x.start + x.size / x.rate;

const MenuBar: React.FC<{f: number}> = ({f}) => {
  const uploading = files.some((x) => progress(f, x) > 0 && progress(f, x) < 1);
  return (
    <div
      style={{
        position: "absolute",
        left: 0,
        right: 0,
        top: 0,
        height: 44,
        display: "flex",
        alignItems: "center",
        padding: "0 26px",
        gap: 30,
        fontSize: 19,
        background: "rgba(20,22,30,0.7)",
        backdropFilter: "blur(20px)",
        borderBottom: `1px solid ${C.line}`,
      }}
    >
      <div style={{fontWeight: 800}}>Finder</div>
      {["File", "Edit", "View", "Go", "Window", "Help"].map((m) => (
        <div key={m} style={{color: "#d8dbe6"}}>
          {m}
        </div>
      ))}
      <div style={{flex: 1}} />
      <div style={{width: 40, height: 32, borderRadius: 7, background: "rgba(255,255,255,0.16)", display: "flex", alignItems: "center", justifyContent: "center"}}>
        <svg width="26" height="26" viewBox="0 0 24 24">
          <path d="M7 18h10.5a4 4 0 0 0 .5-7.97A5.5 5.5 0 0 0 7.4 11 3.5 3.5 0 0 0 7 18z" fill="none" stroke="#fff" strokeWidth="1.7" />
          {uploading && <path d="M12 16v-5m0 0-2.2 2.2M12 11l2.2 2.2" stroke="#fff" strokeWidth="1.7" strokeLinecap="round" />}
        </svg>
      </div>
      <div style={{color: "#d8dbe6"}}>Sat 10:41 AM</div>
    </div>
  );
};

const Panel: React.FC<{f: number}> = ({f}) => {
  const {fps} = useVideoConfig();
  const open = spring({frame: f - 6, fps, config: {damping: 200}});
  const active = files.filter((x) => f < doneAt(x));
  const finished = files.filter((x) => f >= doneAt(x)).sort((a, b) => doneAt(b) - doneAt(a));
  const left = active.reduce((s, x) => s + x.size * (1 - progress(f, x)), 0);
  return (
    <div
      style={{
        position: "absolute",
        right: 150,
        top: 58,
        width: 560,
        padding: 26,
        borderRadius: 20,
        background: "rgba(34,36,44,0.94)",
        border: `1px solid ${C.line}`,
        boxShadow: "0 40px 110px rgba(0,0,0,0.6)",
        opacity: open,
        transform: `translateY(${(1 - open) * -20}px) scale(1.32)`,
        transformOrigin: "top right",
        fontSize: 20,
      }}
    >
      <div style={{display: "flex", gap: 16, alignItems: "center", marginBottom: 22}}>
        <Img src={staticFile("icon.png")} style={{width: 58, height: 58}} />
        <div>
          <div style={{fontWeight: 800, fontSize: 24}}>Drive Stream</div>
          <div style={{color: C.muted, display: "flex", alignItems: "center", gap: 8}}>
            <div style={{width: 10, height: 10, borderRadius: 5, background: C.green}} /> Mounted · you@example.com
          </div>
        </div>
      </div>
      <div style={{display: "flex", justifyContent: "space-between", fontWeight: 700, marginBottom: 14}}>
        <span>Uploads</span>
        <span style={{color: C.muted, fontWeight: 500}}>{active.length ? `${active.length} left · ${fmtBytes(left)}` : "All caught up"}</span>
      </div>
      {active.map((x) => {
        const p = progress(f, x);
        const secs = Math.max(1, Math.round((x.size * (1 - p)) / x.rate / 1));
        return (
          <div key={x.name} style={{marginBottom: 16}}>
            <div style={{fontWeight: 600}}>{x.name}</div>
            <div style={{height: 7, borderRadius: 4, background: "rgba(255,255,255,0.1)", margin: "8px 0 6px"}}>
              <div style={{width: `${Math.max(1.5, p * 100)}%`, height: "100%", borderRadius: 4, background: p > 0 ? brand : C.dim}} />
            </div>
            <div style={{fontSize: 16, color: C.muted}}>
              {p > 0 ? `${fmtBytes(x.size * p)} of ${fmtBytes(x.size)} · ${(x.rate / 1e6).toFixed(1)} MB/s · ${secs}s left` : `${fmtBytes(x.size)} · starts in 2s`}
            </div>
          </div>
        );
      })}
      {finished.length > 0 && <div style={{height: 1, background: C.line, margin: "10px 0 14px"}} />}
      {finished.map((x) => {
        const pop = spring({frame: f - doneAt(x), fps, config: {damping: 10, stiffness: 160}});
        return (
          <div key={x.name} style={{display: "flex", alignItems: "center", gap: 12, marginBottom: 12, opacity: Math.min(1, pop * 1.4)}}>
            <div style={{transform: `scale(${pop})`}}>
              <Check size={24} />
            </div>
            <div style={{flex: 1, fontWeight: 500}}>{x.name}</div>
            <div style={{fontSize: 14, fontWeight: 800, color: C.green, background: "rgba(63,209,122,0.14)", padding: "3px 10px", borderRadius: 99}}>verified</div>
          </div>
        );
      })}
      <div style={{display: "flex", gap: 12, marginTop: 18}}>
        <div style={{flex: 1, textAlign: "center", padding: "11px 0", borderRadius: 10, background: C.blue, fontWeight: 700}}>Open Drive</div>
        <div style={{flex: 1, textAlign: "center", padding: "11px 0", borderRadius: 10, background: "rgba(255,255,255,0.1)", fontWeight: 700}}>Dashboard</div>
      </div>
    </div>
  );
};

/** A file card dragged in from the left into the panel. */
const Flying: React.FC<{f: number}> = ({f}) => {
  const t = ease(f, 12, 40);
  if (f > 44) return null;
  const x = interpolate(t, [0, 1], [380, 1380]);
  const y = interpolate(t, [0, 1], [760, 300]) - Math.sin(t * Math.PI) * 140;
  return (
    <div
      style={{
        position: "absolute",
        left: x,
        top: y,
        width: 150,
        height: 110,
        borderRadius: 14,
        background: "linear-gradient(135deg,#ff8a5c,#6b3a7a)",
        border: "3px solid rgba(255,255,255,0.85)",
        boxShadow: "0 20px 50px rgba(0,0,0,0.5)",
        transform: `rotate(${(1 - t) * -10}deg) scale(${1 - t * 0.5})`,
        opacity: 1 - ease(f, 36, 44),
      }}
    />
  );
};

export const Uploads: React.FC = () => {
  const f = useCurrentFrame();
  const out = useOut(UPLOADS_DURATION);
  return (
    <Background glow={0.8}>
      <AbsoluteFill style={{opacity: out}}>
        <MenuBar f={f} />
        <Panel f={f} />
        <div style={{position: "absolute", left: 130, top: 330, width: 1020}}>
          <Rise delay={4}>
            <div style={{fontSize: 92, fontWeight: 900, letterSpacing: -3, lineHeight: 1.05}}>Drop files in.</div>
          </Rise>
          <Rise delay={22} style={{marginTop: 24}}>
            <div style={{fontSize: 46, fontWeight: 700, color: C.muted, letterSpacing: -0.5}}>They upload in the background.</div>
          </Rise>
          <Rise delay={70} style={{marginTop: 46, display: "flex", alignItems: "center", gap: 20}}>
            <Check size={46} />
            <div style={{fontSize: 38, fontWeight: 700, letterSpacing: -0.5, lineHeight: 1.3}}>
              Every upload is checked against
              <br />
              <GradientText>Google's own checksum.</GradientText>
            </div>
          </Rise>
        </div>
        <Flying f={f} />
        <div style={{position: "absolute", right: 150, bottom: 70, fontSize: 18, color: C.dim, letterSpacing: 1, fontWeight: 700}}>
          TIME-LAPSE · SPED UP 30×
        </div>
      </AbsoluteFill>
    </Background>
  );
};
