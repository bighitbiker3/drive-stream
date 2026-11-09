import React from "react";
import {Audio, interpolate, staticFile} from "remotion";
import {linearTiming, TransitionSeries} from "@remotion/transitions";
import {fade} from "@remotion/transitions/fade";
import {slide} from "@remotion/transitions/slide";
import {wipe} from "@remotion/transitions/wipe";
import {Problem, PROBLEM_DURATION} from "./scenes/Problem";
import {Reveal, REVEAL_DURATION} from "./scenes/Reveal";
import {HowItWorks, HOW_DURATION} from "./scenes/HowItWorks";
import {Speed, SPEED_DURATION} from "./scenes/Speed";
import {Creative, CREATIVE_DURATION} from "./scenes/Creative";
import {Uploads, UPLOADS_DURATION} from "./scenes/Uploads";
import {Dashboard, DASHBOARD_DURATION} from "./scenes/Dashboard";
import {Features, FEATURES_DURATION} from "./scenes/Features";
import {Outro, OUTRO_DURATION} from "./scenes/Outro";
import {TOTAL, TRANSITION} from "./timeline";

export type PromoProps = {
  /** Shown in the closing scene. */
  repo: string;
  /** Optional soundtrack: a file in public/, e.g. "music.mp3". */
  music?: string;
};

const timing = linearTiming({durationInFrames: TRANSITION});
export const PROMO_DURATION = TOTAL;

export const Promo: React.FC<PromoProps> = ({repo, music}) => (
  <>
  {music ? (
    <Audio
      src={staticFile(music)}
      volume={(f) =>
        interpolate(f, [0, 30, PROMO_DURATION - 60, PROMO_DURATION], [0, 0.8, 0.8, 0], {extrapolateLeft: "clamp", extrapolateRight: "clamp"})
      }
    />
  ) : null}
  <TransitionSeries>
    <TransitionSeries.Sequence durationInFrames={PROBLEM_DURATION}>
      <Problem />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition presentation={fade()} timing={timing} />
    <TransitionSeries.Sequence durationInFrames={REVEAL_DURATION}>
      <Reveal />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition presentation={slide({direction: "from-bottom"})} timing={timing} />
    <TransitionSeries.Sequence durationInFrames={HOW_DURATION}>
      <HowItWorks />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition presentation={wipe({direction: "from-left"})} timing={timing} />
    <TransitionSeries.Sequence durationInFrames={SPEED_DURATION}>
      <Speed />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition presentation={slide({direction: "from-bottom"})} timing={timing} />
    <TransitionSeries.Sequence durationInFrames={CREATIVE_DURATION}>
      <Creative />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition presentation={fade()} timing={timing} />
    <TransitionSeries.Sequence durationInFrames={UPLOADS_DURATION}>
      <Uploads />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition presentation={slide({direction: "from-right"})} timing={timing} />
    <TransitionSeries.Sequence durationInFrames={DASHBOARD_DURATION}>
      <Dashboard />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition presentation={fade()} timing={timing} />
    <TransitionSeries.Sequence durationInFrames={FEATURES_DURATION}>
      <Features />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition presentation={fade()} timing={timing} />
    <TransitionSeries.Sequence durationInFrames={OUTRO_DURATION}>
      <Outro repo={repo} />
    </TransitionSeries.Sequence>
  </TransitionSeries>
  </>
);
