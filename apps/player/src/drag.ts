type Invoke = <T = unknown>(command: string, args?: Record<string, unknown>) => Promise<T>;
export function installCharacterDrag(
  canvas: HTMLCanvasElement,
  invoke: Invoke,
  onError: (error: unknown) => void,
  canGrab = (_event: PointerEvent) => true,
) {
  type Gesture = {
    x: number; y: number; grab: { x: number; y: number }; id: number;
    started: boolean; session: string | null; moved: boolean; flushQueued: boolean; cancelled: boolean;
  };
  let pointer: Gesture | null = null, operation = Promise.resolve();
  const enqueue = (work: () => Promise<void>) => { operation = operation.then(work).catch(onError); };
  const queueMotion = (gesture: Gesture) => {
    gesture.moved = true;
    if (gesture.flushQueued) return;
    gesture.flushQueued = true;
    enqueue(async () => {
      try {
        while (!gesture.cancelled && gesture.moved) {
          gesture.moved = false;
          if (gesture.session) await invoke("update_character_edit", { sessionId: gesture.session });
        }
      } finally { gesture.flushQueued = false; }
    });
  };
  canvas.onpointerdown = (event) => {
    if (event.button !== 0 || pointer || !canGrab(event)) return;
    const rect = canvas.getBoundingClientRect();
    pointer = {
      x: event.screenX, y: event.screenY, grab: { x: event.clientX - rect.left, y: event.clientY - rect.top },
      id: event.pointerId, started: false, session: null, moved: false, flushQueued: false, cancelled: false,
    };
    canvas.setPointerCapture(event.pointerId);
  };
  const move = (gesture: Gesture, event: PointerEvent) => {
    const dx = event.screenX - gesture.x, dy = event.screenY - gesture.y;
    if (!gesture.started) {
      if (Math.hypot(dx, dy) < 3) return;
      gesture.started = true;
      enqueue(async () => { gesture.session = await invoke<string>("begin_character_drag", { grab: gesture.grab }); });
    }
    gesture.x = event.screenX; gesture.y = event.screenY;
    queueMotion(gesture);
  };
  canvas.onpointermove = (event) => {
    if (pointer && event.pointerId === pointer.id) move(pointer, event);
  };
  const finish = (event: PointerEvent, cancel: boolean) => {
    const gesture = pointer;
    if (!gesture || event.pointerId !== gesture.id) return;
    if (!cancel && (event.screenX !== gesture.x || event.screenY !== gesture.y)) {
      move(gesture, event);
    }
    pointer = null; gesture.cancelled = cancel;
    if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    if (gesture.started) {
      enqueue(async () => {
        if (gesture.session) await invoke(cancel ? "cancel_character_edit" : "finish_character_edit", { sessionId: gesture.session });
      });
    } else if (!cancel) {
      enqueue(async () => { await invoke("character_click"); });
    }
  };
  canvas.onpointerup = (event) => finish(event, false);
  canvas.onpointercancel = canvas.onlostpointercapture = (event) => finish(event, true);
}
