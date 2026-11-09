import React from "react";
import {AbsoluteFill, Easing, interpolate, spring, useCurrentFrame, useVideoConfig} from "remotion";
import {brand, C, fontFamily} from "./theme";

/** Deep background with slowly drifting brand-colored light. */
export const Background: React.FC<{glow?: number; children?: React.ReactNode}> = ({glow = 1, children}) => {
  const f = useCurrentFrame();
  const t = f / 30;
  const blob = (x: number, y: number, r: number, color: string, o: number) => (
    <div
      style={{
        position: "absolute",
        left: `${x}%`,
        top: `${y}%`,
        width: r,
        height: r,
        marginLeft: -r / 2,
        marginTop: -r / 2,
        borderRadius: "50%",
        background: color,
        filter: "blur(120px)",
        opacity: o * glow,
      }}
    />
  );
  return (
    <AbsoluteFill style={{background: C.bg, fontFamily, color: C.text, overflow: "hidden"}}>
      {blob(18 + 6 * Math.sin(t * 0.35), 22 + 5 * Math.cos(t * 0.3), 900, C.cyan, 0.16)}
      {blob(82 + 5 * Math.cos(t * 0.25), 70 + 6 * Math.sin(t * 0.32), 1000, C.indigo, 0.22)}
      {blob(55 + 8 * Math.sin(t * 0.2), 105, 900, C.blue, 0.14)}
      <AbsoluteFill
        style={{
          backgroundImage:
            "linear-gradient(rgba(255,255,255,0.025) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,0.025) 1px, transparent 1px)",
          backgroundSize: "64px 64px",
          maskImage: "radial-gradient(ellipse at center, black 30%, transparent 75%)",
        }}
      />
      {children}
    </AbsoluteFill>
  );
};

/** Springs in from below with a little blur. */
export const Rise: React.FC<{delay?: number; distance?: number; children: React.ReactNode; style?: React.CSSProperties}> = ({
  delay = 0,
  distance = 40,
  children,
  style,
}) => {
  const f = useCurrentFrame();
  const {fps} = useVideoConfig();
  const p = spring({frame: f - delay, fps, config: {damping: 200, mass: 0.6}});
  return (
    <div
      style={{
        opacity: p,
        transform: `translateY(${(1 - p) * distance}px)`,
        filter: `blur(${(1 - p) * 10}px)`,
        ...style,
      }}
    >
      {children}
    </div>
  );
};

export const GradientText: React.FC<{children: React.ReactNode; style?: React.CSSProperties}> = ({children, style}) => (
  <span
    style={{
      background: brand,
      WebkitBackgroundClip: "text",
      backgroundClip: "text",
      color: "transparent",
      ...style,
    }}
  >
    {children}
  </span>
);

/** Fade the whole scene out over its last frames. */
export const useOut = (duration: number, length = 12) => {
  const f = useCurrentFrame();
  return interpolate(f, [duration - length, duration], [1, 0], {extrapolateLeft: "clamp", extrapolateRight: "clamp"});
};

export const ease = (f: number, from: number, to: number, a = 0, b = 1) =>
  interpolate(f, [from, to], [a, b], {extrapolateLeft: "clamp", extrapolateRight: "clamp", easing: Easing.bezier(0.22, 1, 0.36, 1)});

export const fmtBytes = (n: number) =>
  n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : n >= 1e6 ? `${Math.round(n / 1e6)} MB` : `${Math.round(n / 1e3)} KB`;

export const Check: React.FC<{size?: number; color?: string}> = ({size = 22, color = C.green}) => (
  <svg width={size} height={size} viewBox="0 0 24 24">
    <circle cx="12" cy="12" r="11" fill={color} />
    <path d="M7 12.5l3.2 3.2L17.5 8.5" stroke="#06210f" strokeWidth="2.6" fill="none" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);
