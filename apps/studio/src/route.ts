export const tabs = [
  "overview",
  "resources",
  "animations",
  "events",
  "timing",
  "builds",
] as const;
export type StudioRoute = {
  projectId: string | null;
  tab: (typeof tabs)[number];
  clip: string;
  bindingId: string | null;
};
const safeName = (value: string | null) =>
  value && /^[a-zA-Z0-9_-]{1,64}$/.test(value) ? value : null;
export function readRoute(url: URL): StudioRoute {
  const p = url.searchParams;
  const id = p.get("project");
  return {
    projectId:
      id &&
      id.length <= 128 &&
      (/^[a-z][a-z0-9]*(?:\.[a-z][a-z0-9]*){2,}$/.test(id) ||
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(
          id,
        ))
        ? id
        : null,
    tab: tabs.includes(p.get("tab") as StudioRoute["tab"])
      ? (p.get("tab") as StudioRoute["tab"])
      : "overview",
    clip: safeName(p.get("clip")) ?? "idle",
    bindingId: safeName(p.get("event")),
  };
}
export function writeRoute(url: URL, route: StudioRoute): URL {
  const result = new URL(url);
  if (route.projectId) result.searchParams.set("project", route.projectId);
  else result.searchParams.delete("project");
  result.searchParams.set("tab", route.tab);
  result.searchParams.set("clip", route.clip);
  if (route.bindingId) result.searchParams.set("event", route.bindingId);
  else result.searchParams.delete("event");
  return result;
}
