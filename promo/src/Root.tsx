import React from "react";
import {Composition} from "remotion";
import {Promo, PROMO_DURATION, PromoProps} from "./Promo";
import {FPS} from "./theme";

export const RemotionRoot: React.FC = () => (
  <Composition
    id="DriveStreamPromo"
    component={Promo}
    durationInFrames={PROMO_DURATION}
    fps={FPS}
    width={1920}
    height={1080}
    defaultProps={{repo: "github.com/bighitbiker3/drive-stream", music: "music.wav"} satisfies PromoProps}
  />
);
