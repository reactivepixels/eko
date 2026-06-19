import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";

/**
 * Scrolls its text horizontally when it overflows. By default it scrolls on its own; with
 * `hover`, it stays still (soft right-edge fade) and only scrolls when an ancestor `.card`
 * is hovered.
 */
export function Marquee({
  text,
  className,
  hover,
}: {
  text: string;
  className?: string;
  hover?: boolean;
}) {
  const wrap = useRef<HTMLDivElement>(null);
  const inner = useRef<HTMLSpanElement>(null);
  const [dist, setDist] = useState(0);

  const measure = () => {
    const w = wrap.current,
      i = inner.current;
    if (!w || !i) return;
    const over = i.scrollWidth - w.clientWidth;
    setDist(over > 4 ? over : 0);
  };
  useLayoutEffect(measure, [text]);
  useEffect(() => {
    const ro = new ResizeObserver(measure);
    if (wrap.current) ro.observe(wrap.current);
    return () => ro.disconnect();
  }, []);

  const dur = Math.max(7, dist / 28 + 5); // ~28px/s plus end pauses
  const style: CSSProperties | undefined =
    dist > 0
      ? ({ ["--mqdist" as string]: `${dist}px`, ["--mqdur" as string]: `${dur}s` } as CSSProperties)
      : undefined;

  const wrapClass = ["marquee", hover ? "mq-hover" : "", dist > 0 ? "mq-over" : "", className ?? ""]
    .filter(Boolean)
    .join(" ");
  return (
    <div ref={wrap} className={wrapClass}>
      <span
        ref={inner}
        className={dist > 0 ? "marquee-inner scroll" : "marquee-inner"}
        style={style}
      >
        {text}
      </span>
    </div>
  );
}
