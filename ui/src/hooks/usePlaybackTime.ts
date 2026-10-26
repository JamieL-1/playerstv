import { useEffect, useState, type RefObject } from "react";

/**
 * The video's current time, following it once per presented frame while
 * `playing` (`timeupdate` only fires ~4×/s) and on every seek. Only the
 * component calling this re-renders that often, so keep it small: the seek
 * bar or a time label, not the whole page.
 */
export function usePlaybackTime(video: RefObject<HTMLVideoElement | null>, playing: boolean): number {
  const [time, setTime] = useState(() => video.current?.currentTime ?? 0);
  useEffect(() => {
    const v = video.current;
    if (!v) return;
    const sync = () => setTime(v.currentTime);
    sync();
    // `seeking` shows a drag's new position before the frame has loaded.
    const events = ["seeking", "seeked", "timeupdate"] as const;
    for (const e of events) v.addEventListener(e, sync);
    let frame: number | null = null;
    if (playing) {
      const tick = () => {
        sync();
        frame = v.requestVideoFrameCallback(tick);
      };
      frame = v.requestVideoFrameCallback(tick);
    }
    return () => {
      for (const e of events) v.removeEventListener(e, sync);
      if (frame !== null) v.cancelVideoFrameCallback(frame);
    };
  }, [video, playing]);
  return time;
}
