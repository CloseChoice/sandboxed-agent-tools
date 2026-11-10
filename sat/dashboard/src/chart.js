export const colors = [
  '#8270f4',
  '#47bfa9',
  '#f5ad5c',
  '#69a9ee',
  '#ee809d',
  '#a28acb',
  '#82b55b',
  '#dba85f',
];
export function slices(tools) {
  const total = tools.reduce((sum, tool) => sum + tool.count, 0);
  let angle = -Math.PI / 2;
  const point = (a) => [160 + Math.cos(a) * 132, 160 + Math.sin(a) * 132];
  return tools
    .filter((tool) => tool.count > 0)
    .map((tool, i) => {
      const sweep = (tool.count / total) * Math.PI * 2;
      const end = angle + sweep;
      const [sx, sy] = point(angle);
      const [ex, ey] = point(end);
      let d;
      if (sweep >= Math.PI * 2 - 1e-9) {
        d = 'M 160 28 A 132 132 0 1 1 160 292 A 132 132 0 1 1 160 28 Z';
      } else {
        d = `M 160 160 L ${sx} ${sy} A 132 132 0 ${sweep > Math.PI ? 1 : 0} 1 ${ex} ${ey} Z`;
      }
      const mid = angle + sweep / 2;
      angle = end;
      return {
        ...tool,
        d,
        color: colors[i % colors.length],
        percent: (tool.count / total) * 100,
        translate: `${Math.cos(mid) * 7}px, ${Math.sin(mid) * 7}px`,
      };
    });
}
export function duration(ms) {
  if (ms == null) return 'In progress';
  if (ms < 1000) return `${ms} ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(2)} s`;
  return `${Math.floor(ms / 60000)}m ${Math.round((ms % 60000) / 1000)}s`;
}
export function commandText(tool, args) {
  const quote = (value) =>
    /^[a-zA-Z0-9_./:=@+-]+$/.test(value)
      ? value
      : `'${value.replaceAll("'", "'\\''")}'`;
  return ['sat', tool, ...args].map(quote).join(' ');
}
