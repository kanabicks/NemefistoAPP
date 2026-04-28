/**
 * Фоновые fixed-слои поверх viewport: сетка, виньетка, scanline-overlay.
 * Не имеют интерактивности и pointer-events: none — никаких кликов не ловят.
 */
export function BackgroundLayers() {
  return (
    <>
      <div className="grid-bg" />
      <div className="vignette" />
      <div className="scanlines" />
    </>
  );
}
