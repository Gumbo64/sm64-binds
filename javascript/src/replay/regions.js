// Annotated frame ranges of a replay: a list of
//   { from, to?, label?, note?, color? }
// `to` defaults to the next region's `from` (or the end of the replay);
// `note` and `color` are optional. Where the list comes from (a replay index,
// a file beside the replay) is up to the app.

const COLORS = ['#60a5fa', '#f59e0b', '#a78bfa', '#34d399', '#f472b6', '#22d3ee'];

/** Sorted, with `to` filled in and a colour each; drops entries without a `from`. */
export function normalizeRegions(list, total) {
  const sorted = (Array.isArray(list) ? list : [])
    .filter((r) => Number.isFinite(r?.from))
    .sort((a, b) => a.from - b.from);
  return sorted.map((r, i) => ({
    from: Math.max(0, r.from),
    to: Math.min(total, Number.isFinite(r.to) ? r.to : (sorted[i + 1]?.from ?? total)),
    label: r.label ?? `Region ${i + 1}`,
    note: r.note,
    color: r.color ?? COLORS[i % COLORS.length],
  }));
}

/** The region `frame` is in, or null. */
export const regionAt = (regions, frame) => regions.find((r) => frame >= r.from && frame < r.to) ?? null;
