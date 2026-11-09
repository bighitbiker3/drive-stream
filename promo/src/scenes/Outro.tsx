import React from "react";
import {AbsoluteFill, Img, interpolate, spring, staticFile, useCurrentFrame, useVideoConfig} from "remotion";
import {Background, GradientText, Rise} from "../components";
import {C, mono} from "../theme";
import {dur} from "../timeline";

export const OUTRO_DURATION = dur("outro");

const Chip: React.FC<{children: React.ReactNode}> = ({children}) => (
  <div style={{padding: "10px 22px", borderRadius: 99, border: `1px solid ${C.line}`, background: C.card, fontSize: 26, fontWeight: 600}}>{children}</div>
);

export const Outro: React.FC<{repo: string}> = ({repo}) => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const pop = spring({frame: f - 2, fps, config: {damping: 12, mass: 0.7}});
  const fade = interpolate(f, [OUTRO_DURATION - 20, OUTRO_DURATION], [0, 1], {extrapolateLeft: "clamp", extrapolateRight: "clamp"});
  return (
    <Background glow={1.3}>
      <AbsoluteFill style={{alignItems: "center", justifyContent: "center"}}>
        <div style={{display: "flex", alignItems: "center", gap: 40, marginTop: -60}}>
          <Img
            src={staticFile("icon.png")}
            style={{width: 210, height: 210, transform: `scale(${0.6 + pop * 0.4})`, filter: "drop-shadow(0 0 60px rgba(58,123,255,0.6))"}}
          />
          <Rise delay={8} distance={30}>
            <div style={{fontSize: 120, fontWeight: 900, letterSpacing: -4, lineHeight: 1}}>Drive Stream</div>
            <div style={{fontSize: 44, fontWeight: 700, marginTop: 14, color: C.muted}}>
              Free and <GradientText>open source</GradientText> for macOS.
            </div>
          </Rise>
        </div>
        <Rise delay={26} style={{display: "flex", gap: 18, marginTop: 56}}>
          <Chip>MIT License</Chip>
          <Chip>macOS 14+</Chip>
          <Chip>Rust + SwiftUI</Chip>
          <Chip>No kernel extension</Chip>
        </Rise>
        <Rise delay={40} style={{marginTop: 54}}>
          <div style={{fontFamily: mono, fontSize: 40, padding: "18px 34px", borderRadius: 16, background: "rgba(255,255,255,0.08)", border: `1px solid ${C.line}`}}>
            {repo}
          </div>
        </Rise>
        <Rise delay={54} style={{marginTop: 30}}>
          <div style={{fontSize: 28, color: C.muted}}>Set it up with one prompt to your AI agent.</div>
        </Rise>
      </AbsoluteFill>
      <AbsoluteFill style={{background: "#000", opacity: fade}} />
    </Background>
  );
};
