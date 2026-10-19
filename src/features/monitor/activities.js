import {num} from '../../presentation.js';

// Counts are identified rounds from the archive, never raw notification counts.
export function activityView(anchor, kind, now = Date.now() / 1000) {
  const name = kind === 'red_packet' ? '红包' : '福袋';
  const value = anchor.activities?.[kind];
  if (!value || !Number.isSafeInteger(value.total) || value.total < 0) {
    return {count:'—', state:'unknown', label:`${name}尚未核验`};
  }
  const count = value.unidentified ? value.total ? `≥${num(value.total)}` : '—' : num(value.total);
  const coverage = value.unidentified ? '；部分通知缺少活动标识，未计入次数' : '';
  if (!anchor.active || !['connected','monitoring'].includes(anchor.state)) {
    return {count, state:'unknown', label:`${name}当前状态待核验${coverage}`};
  }
  if (Number.isFinite(value.active_until) && value.active_until > now) {
    return {count, state:'active', label:`${name}进行中${coverage}`};
  }
  if (value.uncertain || value.unidentified) {
    return {count, state:'unknown', label:`${name}已收到通知，结束时间未确认${coverage}`};
  }
  return {count, state:'idle', label:`未发现进行中的${name}${coverage}`};
}
