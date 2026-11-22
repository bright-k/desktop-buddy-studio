export function fitOverlayCanvas(width: number, height: number, viewportWidth: number, viewportHeight: number) {
  // A hidden WebView can report a zero viewport before its first frame.
  const availableWidth = viewportWidth || width;
  const availableHeight = viewportHeight || height;
  const scale = Math.min(1, availableWidth / width, availableHeight / height);
  return {
    scale,
    width: width * scale,
    height: height * scale,
    fractionX: Math.min(1, width * scale / availableWidth),
    fractionY: Math.min(1, height * scale / availableHeight),
  };
}
