import { useEffect, useRef } from "react";

const TRAIL_LEN = 12;

/**
 * Кастомный курсор: внешнее кольцо (lerped, плавно догоняет мышь) +
 * центральная точка (мгновенно) + хвост из 12 затухающих точек.
 *
 * Адаптировано из nemefisto.online (app.jsx CustomCursor):
 * - mix-blend-mode: difference инвертирует цвет под курсором —
 *   видно и на белой кнопке, и на чёрном фоне без переключения palette;
 * - hover-state на интерактивных элементах (data-cursor="hover" или
 *   button/a) увеличивает кольцо до 40px со скруглением 6px;
 * - на touch-устройствах не рендерим вообще (но Tauri-десктоп всё равно
 *   pointer-fine, на всякий случай).
 *
 * Все позиции выставляются через прямые DOM-мутации (refs), а не state —
 * иначе React будет ре-рендерить на каждый mousemove (60fps re-render
 * всего дерева). Здесь мутации точечные, без перерисовки.
 */
export function CustomCursor() {
  const ringRef = useRef<HTMLDivElement | null>(null);
  const dotRef = useRef<HTMLDivElement | null>(null);
  const trailRefs = useRef<(HTMLDivElement | null)[]>([]);
  const trailPos = useRef(
    Array.from({ length: TRAIL_LEN }, () => ({ x: -100, y: -100 }))
  );

  useEffect(() => {
    // Touch-устройство — кастомный курсор не нужен.
    if (
      window.matchMedia("(hover: none)").matches ||
      window.matchMedia("(pointer: coarse)").matches
    ) {
      return;
    }

    let mx = -100;
    let my = -100;
    let rx = -100;
    let ry = -100;
    let hovering = false;

    const onMove = (e: MouseEvent) => {
      mx = e.clientX;
      my = e.clientY;
    };

    // Hover-detect через event delegation на document — ловим
    // mouseover/mouseout от любого вложенного элемента.
    const isInteractive = (el: Element | null): boolean => {
      while (el && el !== document.body) {
        if (el instanceof HTMLElement) {
          const tag = el.tagName;
          if (
            tag === "BUTTON" ||
            tag === "A" ||
            tag === "INPUT" ||
            tag === "SELECT" ||
            tag === "TEXTAREA" ||
            tag === "LABEL" ||
            el.dataset.cursor === "hover"
          ) {
            return true;
          }
        }
        el = el.parentElement;
      }
      return false;
    };

    const onOver = (e: MouseEvent) => {
      hovering = isInteractive(e.target as Element);
      if (ringRef.current) {
        ringRef.current.classList.toggle("is-hover", hovering);
      }
    };

    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseover", onOver);

    let raf = 0;
    const tick = () => {
      raf = requestAnimationFrame(tick);
      // Smooth follow для кольца (lerp 0.18)
      rx += (mx - rx) * 0.18;
      ry += (my - ry) * 0.18;
      if (ringRef.current) {
        ringRef.current.style.transform = `translate(${rx}px, ${ry}px) translate(-50%, -50%)`;
      }
      // Центральная точка — мгновенно
      if (dotRef.current) {
        dotRef.current.style.transform = `translate(${mx}px, ${my}px) translate(-50%, -50%)`;
      }
      // Хвост: каждый кадр сдвигаем массив, точки уменьшаются и тускнеют
      trailPos.current.unshift({ x: mx, y: my });
      trailPos.current.length = TRAIL_LEN;
      trailRefs.current.forEach((el, i) => {
        if (!el) return;
        const p = trailPos.current[i];
        if (!p) return;
        const scale = 1 - i / TRAIL_LEN;
        el.style.transform = `translate(${p.x}px, ${p.y}px) translate(-50%, -50%) scale(${scale})`;
        el.style.opacity = String(scale * 0.35);
      });
    };
    tick();

    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseover", onOver);
    };
  }, []);

  return (
    <>
      {Array.from({ length: TRAIL_LEN }).map((_, i) => (
        <div
          key={i}
          ref={(el) => {
            trailRefs.current[i] = el;
          }}
          className="cursor-trail"
        />
      ))}
      <div ref={dotRef} className="cursor-dot" />
      <div ref={ringRef} className="cursor" />
    </>
  );
}
