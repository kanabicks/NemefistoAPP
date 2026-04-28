import { useEffect, useState } from "react";

/** UTC-часы `HH:MM:SS UTC`, обновляются раз в секунду. */
export function useUtcClock(): string {
  const [time, setTime] = useState("");
  useEffect(() => {
    const tick = () => {
      const d = new Date();
      const pad = (n: number) => String(n).padStart(2, "0");
      setTime(
        `${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(
          d.getUTCSeconds()
        )} UTC`
      );
    };
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, []);
  return time;
}
