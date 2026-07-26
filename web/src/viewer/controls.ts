import { ORBIT_RAD_PER_PIXEL, orbitBy, panBy, zoomBy } from './camera.ts';
import type { CameraHost } from './camera.ts';

/** What input wiring needs beyond the camera it drives. */
export interface ControlHost extends CameraHost {
  setAutoRotate(enabled: boolean): void;
  _lastPinch: { dist: number; midX: number; midY: number } | null;
}

/** A drag gesture, decided once on pointerdown. */
type DragMode = 'orbit' | 'pan' | 'zoom' | null;

/**
 * Pointer, wheel and keyboard wiring for the viewport.
 *
 * Everything here translates input events into camera operations; it owns no
 * camera state of its own. The keys below are claimed while the viewport has
 * focus so they never scroll the page underneath it.
 */
export const NAV_KEYS = new Set([
  'KeyW', 'KeyA', 'KeyS', 'KeyD', 'KeyQ', 'KeyE',
  'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight',
]);

export function installViewerControls(host: ControlHost) {
  const el = host.canvas;
  let lastX = 0, lastY = 0;
  // Drag mode picked once on pointerdown: 'orbit' | 'pan' | 'zoom'.
  let mode: DragMode = null;
  const pointers = new Map<number, { x: number; y: number }>();

  const updateFromPointers = () => {
    if (pointers.size === 1) {
      const [ptr] = pointers.values();
      orbitBy(host, (ptr.x - lastX) * ORBIT_RAD_PER_PIXEL, (ptr.y - lastY) * ORBIT_RAD_PER_PIXEL);
      lastX = ptr.x;
      lastY = ptr.y;
    } else if (pointers.size === 2) {
      const pts = [...pointers.values()];
      const dx = pts[0].x - pts[1].x;
      const dy = pts[0].y - pts[1].y;
      const dist = Math.hypot(dx, dy);
      const midX = (pts[0].x + pts[1].x) * 0.5;
      const midY = (pts[0].y + pts[1].y) * 0.5;
      if (host._lastPinch) {
        zoomBy(host, host._lastPinch.dist / (dist || 1), midX, midY);
        panBy(host, midX - host._lastPinch.midX, midY - host._lastPinch.midY);
      }
      host._lastPinch = { dist, midX, midY };
    }
  };

  el.addEventListener('pointerdown', (e: PointerEvent) => {
    el.setPointerCapture(e.pointerId);
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    lastX = e.clientX;
    lastY = e.clientY;
    if (pointers.size === 1) {
      if (e.button === 1 || e.button === 2 || e.shiftKey) mode = 'pan';
      else if (e.ctrlKey || e.altKey || e.metaKey) mode = 'zoom';
      else mode = 'orbit';
    }
    // Keyboard navigation follows viewport focus.
    el.focus({ preventScroll: true });
    host.setAutoRotate(false);
    host._lastPinch = null;
    e.preventDefault();
  });
  el.addEventListener('pointermove', (e: PointerEvent) => {
    if (!pointers.has(e.pointerId)) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.size === 1 && mode !== 'orbit') {
      const dx = e.clientX - lastX;
      const dy = e.clientY - lastY;
      if (mode === 'pan') panBy(host, dx, dy);
      else if (mode === 'zoom') zoomBy(host, Math.exp(dy * 0.005));
      lastX = e.clientX;
      lastY = e.clientY;
    } else {
      updateFromPointers();
      if (pointers.size === 1) {
        lastX = e.clientX;
        lastY = e.clientY;
      }
    }
    e.preventDefault();
  });
  const endPointer = (e: PointerEvent) => {
    pointers.delete(e.pointerId);
    if (pointers.size < 2) host._lastPinch = null;
    if (pointers.size === 0) mode = null;
    try { el.releasePointerCapture(e.pointerId); } catch (_) { /* ignore */ }
  };
  el.addEventListener('pointerup', endPointer);
  el.addEventListener('pointercancel', endPointer);
  el.addEventListener('pointerleave', endPointer);

  el.addEventListener('contextmenu', (e: Event) => e.preventDefault());
  // Keep the middle button from starting the browser's autoscroll mode.
  el.addEventListener('auxclick', (e) => e.preventDefault());

  el.addEventListener(
    'wheel',
    (e) => {
      e.preventDefault();
      // Firefox reports lines (and pages) rather than pixels; without
      // this the same notch would barely move the camera there.
      let dy = e.deltaY;
      if (e.deltaMode === 1) dy *= 16;
      else if (e.deltaMode === 2) dy *= host.canvas.clientHeight || 400;
      zoomBy(host, Math.exp(dy * 0.001), e.clientX, e.clientY);
    },
    { passive: false },
  );

  el.addEventListener('keydown', (e: KeyboardEvent) => {
    host._navFast = e.shiftKey;
    host._navSlow = e.altKey;
    if (!NAV_KEYS.has(e.code)) return;
    e.preventDefault();
    host.setAutoRotate(false);
    host._navKeys.add(e.code);
  });
  el.addEventListener('keyup', (e: KeyboardEvent) => {
    host._navFast = e.shiftKey;
    host._navSlow = e.altKey;
    host._navKeys.delete(e.code);
  });
  el.addEventListener('blur', () => host._navKeys.clear());
}
