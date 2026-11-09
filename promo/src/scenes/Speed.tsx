import React from "react";
import {AbsoluteFill, spring, useCurrentFrame, useVideoConfig} from "remotion";
import {Background, ease, GradientText, Rise, useOut} from "../components";
import {brand, C, mono} from "../theme";
import {dur} from "../timeline";

export const SPEED_DURATION = dur("speed");

const Row: React.FC<{label: string; value: number; max: number; start: number; f: number; hero?: boolean}> = ({label, value, max, start, f, hero}) => {
  const p = ease(f, start, start + 55);
  const shown = Math.round(value * p);
  return (
    <div style={{marginBottom: 54}}>
      <div style={{display: "flex", alignItems: "baseline", justifyContent: "space-between", marginBottom: 16}}>
        <div style={{fontSize: 36, fontWeight: hero ? 800 : 600, color: hero ? C.text : C.muted}}>{label}</div>
        <div style={{fontFamily: mono, fontSize: hero ? 52 : 40, fontWeight: 800, color: hero ? C.text : C.muted}}>
          {shown} <span style={{fontSize: 26, fontWeight: 600, color: C.muted}}>Mbit/s</span>
        </div>
      </div>
      <div style={{height: hero ? 46 : 34, borderRadius: 23, background: "rgba(255,255,255,0.07)", overflow: "hidden"}}>
        <div
          style={{
            width: `${Math.max(1.2, (value / max) * 100 * p)}%`,
            height: "100%",
            borderRadius: 23,
            background: hero ? brand : "#6b7280",
            boxShadow: hero ? `0 0 40px rgba(58,123,255,${0.6 * p})` : "none",
          }}
        />
      </div>
    </div>
  );
};

export const Speed: React.FC = () => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const out = useOut(SPEED_DURATION);
  const boom = spring({frame: f - 88, fps, config: {damping: 9, mass: 0.7, stiffness: 140}});

  return (
    <Background glow={1}>
      <AbsoluteFill style={{opacity: out}}>
        <Rise delay={2} style={{position: "absolute", top: 90, width: "100%", textAlign: "center"}}>
          <div style={{fontSize: 58, fontWeight: 800, letterSpacing: -1.5}}>Same file. Same connection.</div>
        </Rise>
        <div style={{position: "absolute", left: 260, right: 260, top: 280}}>
          <Row label="Google Drive for desktop" value={13} max={260} start={14} f={f} />
          <Row label="Drive Stream" value={260} max={260} start={26} f={f} hero />
        </div>
        <div
          style={{
            position: "absolute",
            top: 650,
            width: "100%",
            textAlign: "center",
            opacity: Math.min(1, boom * 1.2),
            transform: `scale(${0.6 + boom * 0.4})`,
          }}
        >
          <div style={{fontSize: 150, fontWeight: 900, letterSpacing: -5, lineHeight: 1}}>
            Up to <GradientText>20× faster</GradientText>
          </div>
        </div>
        <Rise delay={112} style={{position: "absolute", bottom: 80, width: "100%", textAlign: "center"}}>
          <div style={{fontSize: 26, color: C.muted}}>
            Measured on a 2 GB 4K clip over one home connection · 220–260 vs 13 Mbit/s · seeks in ~0.5 s
          </div>
        </Rise>
      </AbsoluteFill>
    </Background>
  );
};
