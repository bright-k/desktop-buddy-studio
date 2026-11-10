type HitMask = {
  columns: number; rows: number; bits: number[];
  offsetX: number; offsetY: number; fractionX: number; fractionY: number;
};
const MAX_EDGE = 192;
const MIN_ALPHA = 16;
const empty: HitMask = { columns: 0, rows: 0, bits: [], offsetX: 0, offsetY: 0, fractionX: 1, fractionY: 1 };

export function trackCharacterPixels(
  canvas: HTMLCanvasElement,
  publish: (mask: HitMask) => Promise<unknown>,
  onError: (error: unknown) => void,
) {
  const sample = document.createElement("canvas");
  const context = sample.getContext("2d", { willReadFrequently: true });
  if (!context) throw new Error("클릭 영역을 준비하지 못했습니다.");
  let current = empty;
  let sent = empty;
  let publishing = false;

  async function sendLatest() {
    publishing = true;
    try {
      // Keep at most one update in flight during fast animation or resize.
      while (sent !== current) {
        const next = current;
        await publish(next);
        sent = next;
      }
    } catch (error) {
      onError(error);
    } finally {
      publishing = false;
    }
  }

  function replace(mask: HitMask) {
    if (mask.columns !== current.columns || mask.rows !== current.rows ||
        mask.offsetX !== current.offsetX || mask.offsetY !== current.offsetY ||
        mask.fractionX !== current.fractionX || mask.fractionY !== current.fractionY ||
        (mask.bits !== current.bits && mask.bits.some((word, index) => word !== current.bits[index]))) {
      current = mask;
    }
    if (!publishing && sent !== current) void sendLatest();
  }

  return {
    place(offsetX: number, offsetY: number, fractionX: number, fractionY: number) {
      replace({ ...current, offsetX, offsetY, fractionX, fractionY });
    },
    update(drawn: boolean) {
      if (!drawn) {
        replace({ ...current, columns: 0, rows: 0, bits: [] });
        return;
      }
      const scale = Math.min(1 / (window.devicePixelRatio || 1), MAX_EDGE / Math.max(canvas.width, canvas.height));
      const columns = Math.max(1, Math.round(canvas.width * scale));
      const rows = Math.max(1, Math.round(canvas.height * scale));
      if (sample.width !== columns) sample.width = columns;
      if (sample.height !== rows) sample.height = rows;
      context.clearRect(0, 0, columns, rows);
      try {
        context.drawImage(canvas, 0, 0, columns, rows);
        const { data } = context.getImageData(0, 0, columns, rows);
        const bits = Array<number>(Math.ceil(columns * rows / 32)).fill(0);
        for (let pixel = 0; pixel < columns * rows; pixel++) {
          if (data[pixel * 4 + 3] >= MIN_ALPHA) {
            const word = pixel >>> 5;
            bits[word] = (bits[word] | (1 << (pixel & 31))) >>> 0;
          }
        }
        replace({ ...current, columns, rows, bits });
      } catch (error) {
        replace({ ...current, columns: 0, rows: 0, bits: [] });
        onError(error);
      }
    },
    contains(event: Pick<MouseEvent, "clientX" | "clientY">) {
      const rect = canvas.getBoundingClientRect();
      const x = Math.floor((event.clientX - rect.left) / rect.width * current.columns);
      const y = Math.floor((event.clientY - rect.top) / rect.height * current.rows);
      if (!(x >= 0 && x < current.columns && y >= 0 && y < current.rows)) return false;
      const pixel = y * current.columns + x;
      return (current.bits[pixel >>> 5] & (1 << (pixel & 31))) !== 0;
    },
  };
}
