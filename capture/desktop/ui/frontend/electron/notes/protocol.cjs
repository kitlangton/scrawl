const text = v => typeof v === 'string' && Buffer.byteLength(v) <= 8192 && !v.includes('\0');
const pin = n => Number.isInteger(n) && n > 0 && n <= 100000;
function validCommand(v) {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return false;
  const fields = { draft: ['action', 'pin', 'text'], save: ['action', 'pin', 'text'],
    delete: ['action', 'pin'], edit: ['action', 'pin'], dismiss: ['action'], list: ['action'] }[v.action];
  return fields && Object.keys(v).length === fields.length && Object.keys(v).every(k => fields.includes(k)) &&
    (!fields.includes('pin') || pin(v.pin)) && (!fields.includes('text') || text(v.text));
}
function validSnapshot(v) {
  const note = n => n && pin(n.pin) && Number.isSafeInteger(n.t) && n.t >= 0 &&
    [n.x, n.y].every(p => Number.isFinite(p) && p >= 0 && p <= 1) && text(n.text);
  return v && ['badge', 'editor', 'list'].includes(v.mode) && (v.current === null || pin(v.current)) &&
    (v.badgeRect == null || (Array.isArray(v.badgeRect) && v.badgeRect.length === 4 && v.badgeRect.every(Number.isFinite))) &&
    Array.isArray(v.position) && v.position.length === 2 && v.position.every(Number.isFinite) &&
    Array.isArray(v.notes) && v.notes.length <= 100 && v.notes.every(note) &&
    Array.isArray(v.drafts) && v.drafts.length <= 100 && v.drafts.every(note) &&
    (v.error === null || typeof v.error === 'string');
}
function popupBounds(point, area, height) {
  const width = Math.min(332, area.width);
  return { x: Math.round(Math.max(area.x, Math.min(point.x + 12, area.x + area.width - width))),
    y: Math.round(Math.max(area.y, Math.min(point.y + 12, area.y + area.height - height))),
    width, height: Math.min(height, area.height) };
}
function badgeBounds(rect, area) {
  const [x, y, width, height] = rect;
  let bx = x + width + 6, by = y;
  if (bx + 150 > area.x + area.width) {
    bx = x - 156;
    if (bx < area.x) { bx = x + (width - 150) / 2; by = y + height + 6; }
  }
  return { x: Math.round(Math.max(area.x, Math.min(bx, area.x + area.width - 150))),
    y: Math.round(Math.max(area.y, Math.min(by, area.y + area.height - 44))) };
}
module.exports = { validCommand, validSnapshot, popupBounds, badgeBounds };
