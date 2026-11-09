import React from "react";
import {AbsoluteFill, Img, interpolate, spring, staticFile, useCurrentFrame, useVideoConfig} from "remotion";
import {Background, GradientText, Rise, useOut} from "../components";
import {C, mono} from "../theme";
import {dur} from "../timeline";

export const DASHBOARD_DURATION = dur("dashboard");

export const Dashboard: React.FC = () => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const out = useOut(DASHBOARD_DURATION);
  const enter = spring({frame: f - 4, fps, config: {damping: 200, mass: 0.9}});
  const scroll = interpolate(f, [40, DASHBOARD_DURATION], [0, 210], {extrapolateLeft: "clamp", extrapolateRight: "clamp"});
  const W = 1000;

  return (
    <Background glow={1}>
      <AbsoluteFill style={{opacity: out}}>
        <div style={{position: "absolute", left: 120, top: 330, width: 640}}>
          <Rise delay={10}>
            <div style={{fontSize: 74, fontWeight: 900, letterSpacing: -2.5, lineHeight: 1.05}}>
              Plus a <GradientText>dashboard</GradientText> for everything.
            </div>
          </Rise>
          <Rise delay={28} style={{marginTop: 30}}>
            <div style={{fontSize: 32, color: C.muted, lineHeight: 1.45, fontWeight: 500}}>
              Sign in to Google, watch uploads, check the cache and your API quota.
            </div>
          </Rise>
          <Rise delay={44} style={{marginTop: 22}}>
            <div style={{fontSize: 32, color: C.muted, lineHeight: 1.45, fontWeight: 500}}>
              No terminal required.
            </div>
          </Rise>
        </div>

        <div style={{position: "absolute", left: 820, top: 110, perspective: 1600}}>
          <div
            style={{
              width: W,
              height: 860,
              borderRadius: 18,
              overflow: "hidden",
              background: "#f5f5f7",
              border: `1px solid ${C.line}`,
              boxShadow: "0 50px 140px rgba(0,0,0,0.6), 0 0 80px rgba(58,123,255,0.25)",
              opacity: enter,
              transform: `translateY(${(1 - enter) * 120}px) rotateX(${(1 - enter) * 22}deg) rotateY(${-6 + enter * 2}deg)`,
              transformOrigin: "center bottom",
            }}
          >
            <div style={{height: 52, background: "#e8e8ed", display: "flex", alignItems: "center", gap: 9, padding: "0 18px", borderBottom: "1px solid #d4d4da"}}>
              {["#ff5f57", "#febc2e", "#28c840"].map((c) => (
                <div key={c} style={{width: 13, height: 13, borderRadius: 7, background: c}} />
              ))}
              <div
                style={{
                  marginLeft: 22,
                  flex: 1,
                  height: 32,
                  borderRadius: 8,
                  background: "#fff",
                  display: "flex",
                  alignItems: "center",
                  padding: "0 14px",
                  fontFamily: mono,
                  fontSize: 17,
                  color: "#555",
                }}
              >
                127.0.0.1:7317
              </div>
            </div>
            <div style={{height: 808, overflow: "hidden"}}>
              <Img src={staticFile("dashboard.png")} style={{width: W, transform: `translateY(${-scroll}px)`}} />
            </div>
          </div>
        </div>
      </AbsoluteFill>
    </Background>
  );
};
