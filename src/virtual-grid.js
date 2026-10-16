// The fixed card height and gap come from CSS, so zoom and resizing use the same geometry.
export function gridWindow({ count, width, height, scrollTop, minWidth, cardHeight, gap, overscan = 2 }) {
  const columns = Math.max(1, Math.floor((width + gap) / (minWidth + gap)));
  const totalRows = Math.ceil(count / columns);
  const stride = cardHeight + gap;
  const firstRow = Math.min(Math.max(0, totalRows - 1), Math.max(0, Math.floor(scrollTop / stride) - overscan));
  const lastRow = Math.min(totalRows, Math.ceil((scrollTop + height) / stride) + overscan);
  return {
    columns,
    start: firstRow * columns,
    end: Math.min(count, lastRow * columns),
    top: firstRow * stride,
    bottom: Math.max(0, totalRows - lastRow) * stride,
  };
}

// A section retains its full height even when another section owns the viewport.
// offset is measured from the shared scroll root's content top, including headers.
export function gridSectionWindow({ count, width, height, scrollTop, offset = 0, minWidth, cardHeight, gap, overscan = 2 }) {
  const columns = Math.max(1, Math.floor((width + gap) / (minWidth + gap)));
  const totalRows = Math.ceil(count / columns);
  const stride = cardHeight + gap;
  const totalHeight = totalRows ? totalRows * stride - gap : 0;
  const visibleTop = Math.max(0, scrollTop - offset);
  const visibleBottom = Math.min(totalHeight, scrollTop + height - offset);
  if (!count || height <= 0 || visibleTop >= visibleBottom) {
    return { columns, start: 0, end: 0, top: 0, bottom: totalHeight, totalHeight, visible: false };
  }
  const extraRows = Math.max(0, Math.floor(overscan));
  const firstRow = Math.max(0, Math.floor(visibleTop / stride) - extraRows);
  const lastRow = Math.min(totalRows, Math.ceil(visibleBottom / stride) + extraRows);
  return {
    columns,
    start: firstRow * columns,
    end: Math.min(count, lastRow * columns),
    top: firstRow * stride,
    bottom: (totalRows - lastRow) * stride,
    totalHeight,
    visible: true,
  };
}
