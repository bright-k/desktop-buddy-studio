import type { PreferenceUpdate, RuntimeStatus } from "../../../packages/contracts/generated/index";

export type SettingsValues = Record<string, string | boolean>;
export function settingsValues(status: RuntimeStatus): SettingsValues {
  return {
    keyboard: status.keyboard, mouse: status.mouse, movement: status.movementEnabled,
    "cursor-follow": status.cursorFollow,
    "sound-enabled": status.soundEnabled,
    top: status.alwaysOnTop, scale: String(Math.round(status.scale * 100)), monitor: status.basePlacement.monitorHint,
    "position-x": String(Math.round(status.basePlacement.position.x)), "position-y": String(Math.round(status.basePlacement.position.y)),
    "idle-timeout": String(status.timing.idleTimeoutMs), "rate-stability": String(status.timing.rateStabilityMs),
    "rate-window": String(status.timing.rateWindowMs), "rate-evaluation": String(status.timing.rateEvaluationMs),
  };
}
// Apply external tray/drag changes only where the user has not edited a field.
export function mergeSettings(current: SettingsValues, baseline: SettingsValues, incoming: SettingsValues): SettingsValues {
  return Object.fromEntries(Object.entries(incoming).map(([key, value]) => [key, current[key] === baseline[key] ? value : current[key]]));
}
export function settingsUpdate(values: SettingsValues, baseline: SettingsValues = {}): PreferenceUpdate {
  const number = (key: string, min: number, max: number) => {
    const raw = values[key]; const value = Number(raw);
    if (typeof raw !== "string" || !raw.trim() || !Number.isFinite(value) || value < min || value > max)
      throw new Error("빈 값이나 허용 범위를 벗어난 숫자가 있습니다. 표시된 항목을 확인해주세요.");
    return value;
  };
  const milliseconds = (key: string, min: number, max: number) => {
    const value = number(key, min, max);
    if (!Number.isInteger(value)) throw new Error("반응 시간은 정수 밀리초로 입력해주세요.");
    return value;
  };
  const idleTimeoutMs = milliseconds("idle-timeout", 50, 5000), rateEvaluationMs = milliseconds("rate-evaluation", 10, 250);
  if (rateEvaluationMs > idleTimeoutMs) throw new Error("속도 평가 간격은 입력 중단 시간 이하여야 합니다.");
  if (!values.monitor) throw new Error("연결된 디스플레이를 선택해주세요.");
  const changed = (...keys: string[]) => keys.some((key) => values[key] !== baseline[key]);
  const toggle = (key: string) => changed(key) ? values[key] === true : null;
  return {
    keyboard: toggle("keyboard"), mouse: toggle("mouse"), movementEnabled: toggle("movement"),
    cursorFollow: toggle("cursor-follow"),
    soundEnabled: toggle("sound-enabled"),
    alwaysOnTop: toggle("top"), scale: changed("scale") ? number("scale", 25, 300) / 100 : null,
    timing: changed("idle-timeout", "rate-evaluation", "rate-window", "rate-stability")
      ? { idleTimeoutMs, rateEvaluationMs, rateWindowMs: milliseconds("rate-window", 100, 10000), rateStabilityMs: milliseconds("rate-stability", 0, 2000) }
      : null,
    basePlacement: changed("monitor", "position-x", "position-y")
      ? { monitorHint: String(values.monitor), position: { x: number("position-x", -1000000, 1000000), y: number("position-y", -1000000, 1000000) } }
      : null,
  };
}
