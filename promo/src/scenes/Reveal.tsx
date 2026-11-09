import React from "react";
import {AbsoluteFill, Img, interpolate, spring, staticFile, useCurrentFrame, useVideoConfig} from "remotion";
import {Background, GradientText, Rise, useOut} from "../components";
import {C} from "../theme";
import {dur} from "../timeline";

export const REVEAL_DURATION = dur("reveal");

export const Reveal: React.FC = () => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const out = useOut(REVEAL_DURATION);
  const pop = spring({frame: f - 2, fps, config: {damping: 11, mass: 0.8, stiffness: 120}});
  const ring = (f % 60) / 60;
  const glow = interpolate(f, [0, 25], [0, 1], {extrapolateRight: "clamp"});

  return (
    <Background glow={1.2}>
      <AbsoluteFill style={{alignItems: "center", justifyContent: "center", opacity: out}}>
        <div style={{position: "relative", width: 300, height: 300, marginTop: -150}}>
          {[0, 0.5].map((o) => {
            const r = (ring + o) % 1;
            return (
              <div
                key={o}
                style={{
                  position: "absolute",
                  inset: -r * 120,
                  borderRadius: 90 + r * 60,
                  border: `2px solid ${C.cyan}`,
                  opacity: (1 - r) * 0.35 * glow,
                }}
              />
            );
          })}
          <div
            style={{
              position: "absolute",
              inset: 30,
              borderRadius: 60,
              boxShadow: `0 0 ${140 * glow}px ${30 * glow}px rgba(58,123,255,0.55)`,
            }}
          />
          <Img
            src={staticFile("icon.png")}
            style={{
              width: 300,
              height: 300,
              transform: `scale(${0.5 + pop * 0.5}) rotate(${(1 - pop) * -12}deg)`,
              filter: `blur(${(1 - Math.min(1, pop)) * 12}px)`,
            }}
          />
        </div>
        <Rise delay={16} style={{marginTop: 26}}>
          <div style={{fontSize: 132, fontWeight: 900, letterSpacing: -4, lineHeight: 1}}>Drive Stream</div>
        </Rise>
        <Rise delay={30} style={{marginTop: 22}}>
          <div style={{fontSize: 54, fontWeight: 700, letterSpacing: -1}}>
            Your Google Drive, <GradientText>streamed.</GradientText>
          </div>
        </Rise>
        <Rise delay={46} style={{marginTop: 18}}>
          <div style={{fontSize: 32, color: C.muted, fontWeight: 500}}>Open anything instantly. Nothing downloads whole.</div>
        </Rise>
      </AbsoluteFill>
    </Background>
  );
};
