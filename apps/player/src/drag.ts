type Invoke = <T = unknown>(command: string, args?: Record<string, unknown>) => Promise<T>;
export function installCharacterDrag(canvas: HTMLCanvasElement, invoke: Invoke, onError: (error: unknown) => void) {
  type Gesture = { x: number; y: number; id: number; started: boolean; session: string | null; dx: number; dy: number; flushQueued: boolean; cancelled: boolean };
  let pointer: Gesture | null = null, operation = Promise.resolve();
  const enqueue = (work: () => Promise<void>) => { operation = operation.then(work).catch(onError); };
  canvas.title = "드래그해서 위치 이동 · 크기는 트레이 설정에서 조절";
  canvas.onpointerdown = (event) => {
    if (event.button !== 0 || pointer) return;
    pointer = { x: event.screenX, y: event.screenY, id: event.pointerId, started: false, session: null, dx: 0, dy: 0, flushQueued: false, cancelled: false };
    canvas.setPointerCapture(event.pointerId);
  };
  canvas.onpointermove = (event) => {
    const gesture = pointer;
    if (!gesture || event.pointerId !== gesture.id) return;
    const dx = event.screenX - gesture.x, dy = event.screenY - gesture.y;
    if (!gesture.started) {
      if (Math.hypot(dx, dy) < 3) return;
      gesture.started = true;
      enqueue(async () => { gesture.session = await invoke<string>("begin_character_drag"); });
    }
    gesture.x = event.screenX; gesture.y = event.screenY;
    gesture.dx += dx; gesture.dy += dy;
    if (gesture.flushQueued) return;
    gesture.flushQueued = true;
    enqueue(async () => {
      try {
        while (!gesture.cancelled && (gesture.dx || gesture.dy)) {
          const dx = gesture.dx, dy = gesture.dy;
          gesture.dx = gesture.dy = 0;
          if (gesture.session) await invoke("update_character_edit", { sessionId: gesture.session, dx, dy });
        }
      } finally { gesture.flushQueued = false; }
    });
  };
  const finish = (event: PointerEvent, cancel: boolean) => {
    const gesture = pointer;
    if (!gesture || event.pointerId !== gesture.id) return;
    pointer = null; gesture.cancelled = cancel;
    if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    if (gesture.started) enqueue(async () => {
      if (gesture.session) await invoke(cancel ? "cancel_character_edit" : "finish_character_edit", { sessionId: gesture.session });
    });
  };
  canvas.onpointerup = (event) => finish(event, false);
  canvas.onpointercancel = canvas.onlostpointercapture = (event) => finish(event, true);
}
