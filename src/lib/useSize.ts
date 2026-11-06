import { useLayoutEffect, useState, type RefObject } from "react";

/** The element's client size, kept current with a ResizeObserver (canvas, plots, s-plane). */
export function useSize(ref: RefObject<Element | null>) {
  const [size, setSize] = useState({ w: 0, h: 0 });
  useLayoutEffect(() => {
    const el = ref.current!;
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return size;
}
