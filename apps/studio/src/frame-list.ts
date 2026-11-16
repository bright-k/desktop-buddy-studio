export const frameListLimit = 256;

export function parseFrameList(text: string, frameCount: number): number[] {
  const frames: number[] = [];
  for (const part of text.split(",")) {
    const token = part.trim();
    if (!token) continue;
    const match = /^(\d+)(?:\s*-\s*(\d+))?$/.exec(token);
    if (!match) throw new Error("프레임 번호나 범위를 입력하세요. 예: 0, 8-31, 40");

    const first = Number(match[1]);
    const last = match[2] === undefined ? first : Number(match[2]);
    if (first >= frameCount || last >= frameCount)
      throw new Error(`프레임 번호는 0~${frameCount - 1} 사이여야 합니다.`);

    const length = Math.abs(last - first) + 1;
    if (frames.length + length > frameListLimit)
      throw new Error(`재생 목록은 최대 ${frameListLimit}프레임입니다.`);

    const step = last < first ? -1 : 1;
    for (let offset = 0; offset < length; offset++) frames.push(first + offset * step);
  }
  return frames;
}

export function formatFrameList(frames: readonly number[]): string {
  const parts: string[] = [];
  for (let start = 0; start < frames.length;) {
    let end = start;
    const step = frames[start + 1] - frames[start];
    if (step === 1 || step === -1) {
      while (end + 1 < frames.length && frames[end + 1] - frames[end] === step) end++;
    }
    if (end - start >= 2) {
      parts.push(`${frames[start]}-${frames[end]}`);
      start = end + 1;
    } else {
      parts.push(String(frames[start++]));
    }
  }
  return parts.join(", ");
}
