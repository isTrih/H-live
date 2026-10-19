// Join once, filter before pagination; rendering stays bounded for large pools.
export function proxyAssignments(leases, anchors, query = '', page = 0, size = 50) {
  const names = new Map(anchors.map(a => [a.rid, a.name || a.rid]));
  const q = query.trim().toLowerCase();
  const rows = leases.map(l => ({...l, name:names.get(l.rid) || l.rid})).filter(l =>
    !q || `${l.name} ${l.rid} ${l.proxy || l.endpoint || ''}`.toLowerCase().includes(q));
  size = Math.max(1, Math.min(100, Math.floor(size) || 50));
  const pages = Math.max(1, Math.ceil(rows.length / size));
  page = Math.min(pages - 1, Math.max(0, Math.floor(page) || 0));
  return {items:rows.slice(page * size, (page + 1) * size), total:rows.length, page, pages,
    first:rows.length ? page * size + 1 : 0, last:Math.min(rows.length, (page + 1) * size)};
}
