import React from "react";
import {AbsoluteFill, spring, useCurrentFrame, useVideoConfig} from "remotion";
import {Background, GradientText, Rise, useOut} from "../components";
import {C} from "../theme";
import {dur} from "../timeline";

export const FEATURES_DURATION = dur("features");

const icon = (d: React.ReactNode) => (
  <svg width="54" height="54" viewBox="0 0 24 24" fill="none" stroke="url(#g)" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
    <defs>
      <linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stopColor={C.cyan} />
        <stop offset="1" stopColor={C.indigo} />
      </linearGradient>
    </defs>
    {d}
  </svg>
);

const cards: {title: string; body: string; glyph: React.ReactNode}[] = [
  {title: "No kernel extension", body: "Uses the NFS client already built into macOS.", glyph: icon(<><rect x="4" y="4" width="16" height="16" rx="3" /><path d="M9 12l2 2 4-4" /></>)},
  {title: "Works with every app", body: "Finder, Resolve, Premiere, Ableton, ffmpeg…", glyph: icon(<><rect x="3" y="5" width="18" height="13" rx="2" /><path d="M10 9.5v5l4-2.5z" /></>)},
  {title: "Instant renames & moves", body: "Metadata-only on Drive. Nothing re-uploads.", glyph: icon(<><path d="M4 12h12" /><path d="M12 6l6 6-6 6" /></>)},
  {title: "Synced in seconds", body: "Edits from your phone or the web show up fast.", glyph: icon(<><path d="M20 12a8 8 0 1 1-2.3-5.6" /><path d="M20 4v5h-5" /></>)},
  {title: "Smart local cache", body: "Recent blocks stay on your Mac; the oldest go first.", glyph: icon(<><ellipse cx="12" cy="6" rx="7" ry="3" /><path d="M5 6v6c0 1.7 3.1 3 7 3s7-1.3 7-3V6" /><path d="M5 12v6c0 1.7 3.1 3 7 3s7-1.3 7-3v-6" /></>)},
  {title: "Your own API quota", body: "Bring your own Google OAuth client. No middleman.", glyph: icon(<><circle cx="8" cy="15" r="4" /><path d="M11 12l8-8M16 7l3 3" /></>)},
];

export const Features: React.FC = () => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const out = useOut(FEATURES_DURATION);
  return (
    <Background glow={0.9}>
      <AbsoluteFill style={{opacity: out}}>
        <Rise delay={2} style={{position: "absolute", top: 110, width: "100%", textAlign: "center"}}>
          <div style={{fontSize: 70, fontWeight: 900, letterSpacing: -2}}>
            Feels like a <GradientText>normal folder.</GradientText>
          </div>
        </Rise>
        <div style={{position: "absolute", left: 200, right: 200, top: 300, display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 30}}>
          {cards.map((c, i) => {
            const p = spring({frame: f - 14 - i * 6, fps, config: {damping: 14, mass: 0.6}});
            return (
              <div
                key={c.title}
                style={{
                  height: 260,
                  padding: "34px 34px",
                  borderRadius: 22,
                  background: C.card,
                  border: `1px solid ${C.line}`,
                  opacity: Math.min(1, p),
                  transform: `translateY(${(1 - p) * 50}px) scale(${0.92 + p * 0.08})`,
                }}
              >
                {c.glyph}
                <div style={{fontSize: 34, fontWeight: 800, marginTop: 22, letterSpacing: -0.5}}>{c.title}</div>
                <div style={{fontSize: 24, color: C.muted, marginTop: 12, lineHeight: 1.4}}>{c.body}</div>
              </div>
            );
          })}
        </div>
      </AbsoluteFill>
    </Background>
  );
};
