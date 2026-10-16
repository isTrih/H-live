import { test } from 'node:test';
import assert from 'node:assert/strict';
import { gridWindow, gridSectionWindow } from './virtual-grid.js';

test('continuous grid covers every visible row without gaps across window sizes', () => {
  for (const count of [1, 3, 300, 500]) for (const width of [990, 1310]) {
    const height = 630, cardHeight = 168, gap = 14, stride = cardHeight + gap;
    const columns = Math.floor((width + gap) / (208 + gap));
    const totalRows = Math.ceil(count / columns);
    const fullHeight = totalRows * stride - gap;
    const seen = new Set();
    for (let top = 0; top <= Math.max(0, fullHeight - height) + height; top += 150) {
      const scrollTop = Math.min(top, Math.max(0, fullHeight - height));
      const r = gridWindow({ count, width, height, scrollTop, minWidth: 208, cardHeight, gap });
      const firstVisible = Math.floor(scrollTop / stride) * columns;
      const lastVisible = Math.min(count, Math.ceil((scrollTop + height) / stride) * columns);
      assert.ok(r.start <= firstVisible && r.end >= lastVisible);
      assert.ok(r.end - r.start <= (Math.ceil(height / stride) + 5) * columns);
      const renderedRows = Math.ceil((r.end - r.start) / columns);
      assert.equal(r.top + renderedRows * stride - gap + r.bottom, fullHeight);
      for (let i = r.start; i < r.end; i++) seen.add(i);
    }
    assert.equal(seen.size, count);
  }
});

test('shrinking a filtered list while scrolled returns its remaining cards', () => {
  const r = gridWindow({ count: 3, width: 1310, height: 630, scrollTop: 8000, minWidth: 208, cardHeight: 168, gap: 10 });
  assert.equal(r.start, 0);
  assert.equal(r.end, 3);
  assert.equal(r.top, 0);
  assert.equal(r.bottom, 0);
});

test('two sections share a global scroll position and clip the viewport at their boundaries', () => {
  const geometry = { width: 410, height: 250, minWidth: 200, cardHeight: 100, gap: 10, overscan: 0 };
  const upper = (scrollTop) => gridSectionWindow({ ...geometry, count: 20, scrollTop });
  const lower = (scrollTop) => gridSectionWindow({ ...geometry, count: 12, offset: 1162, scrollTop });
  assert.deepEqual(upper(0), { columns: 2, start: 0, end: 6, top: 0, bottom: 770, totalHeight: 1090, visible: true });
  assert.deepEqual(lower(0), { columns: 2, start: 0, end: 0, top: 0, bottom: 650, totalHeight: 650, visible: false });
  assert.deepEqual(upper(950), { columns: 2, start: 16, end: 20, top: 880, bottom: 0, totalHeight: 1090, visible: true });
  assert.deepEqual(lower(950), { columns: 2, start: 0, end: 2, top: 0, bottom: 550, totalHeight: 650, visible: true });
  assert.equal(upper(1300).visible, false);
  assert.deepEqual(lower(1300), { columns: 2, start: 2, end: 8, top: 110, bottom: 220, totalHeight: 650, visible: true });
});

test('nonintersecting sections have an empty slice and exact spacers even with overscan', () => {
  const geometry = { count: 7, width: 410, height: 200, minWidth: 200, cardHeight: 100, gap: 10, offset: 500, overscan: 2 };
  for (const scrollTop of [0, 300, 930, 5000]) {
    const r = gridSectionWindow({ ...geometry, scrollTop });
    assert.equal(r.visible, false);
    assert.equal(r.start, 0);
    assert.equal(r.end, 0);
    assert.equal(r.top + r.bottom, 430);
    assert.equal(r.totalHeight, 430);
  }
  const empty = gridSectionWindow({ ...geometry, count: 0, scrollTop: 500 });
  assert.deepEqual(empty, { columns: 2, start: 0, end: 0, top: 0, bottom: 0, totalHeight: 0, visible: false });
  const hiddenRoot = gridSectionWindow({ ...geometry, height: 0, scrollTop: 600 });
  assert.equal(hiddenRoot.visible, false);
  assert.equal(hiddenRoot.end, 0);
  assert.equal(hiddenRoot.top + hiddenRoot.bottom, 430);
});

test('section count or column changes preserve geometry before and after the root clamps scroll', () => {
  const geometry = { count: 20, width: 410, height: 250, scrollTop: 900, minWidth: 200, cardHeight: 100, gap: 10 };
  assert.equal(gridSectionWindow(geometry).visible, true);
  const shrunk = gridSectionWindow({ ...geometry, count: 2 });
  assert.equal(shrunk.visible, false);
  assert.equal(shrunk.totalHeight, 100);
  assert.equal(shrunk.start, shrunk.end);
  const countClamped = gridSectionWindow({ ...geometry, count: 2, scrollTop: 0 });
  assert.equal(countClamped.visible, true);
  assert.equal(countClamped.start, 0);
  assert.equal(countClamped.end, 2);
  const widened = gridSectionWindow({ ...geometry, width: 830 });
  assert.equal(widened.columns, 4);
  assert.equal(widened.visible, false);
  assert.equal(widened.totalHeight, 540);
  const widthClamped = gridSectionWindow({ ...geometry, width: 830, scrollTop: 290 });
  assert.equal(widthClamped.visible, true);
  assert.equal(widthClamped.end, 20);
});

test('section overscan stays bounded and preserves exact height at every viewport crossing', () => {
  for (const count of [1, 7, 300, 500]) for (const width of [410, 830]) {
    const height = 250, cardHeight = 100, gap = 10, offset = 413, overscan = 2;
    const columns = Math.floor((width + gap) / 210), stride = cardHeight + gap;
    const totalHeight = Math.ceil(count / columns) * stride - gap;
    for (let scrollTop = 0; scrollTop < offset + totalHeight + height; scrollTop += 37) {
      const r = gridSectionWindow({ count, width, height, scrollTop, offset, minWidth: 200, cardHeight, gap, overscan });
      assert.ok(0 <= r.start && r.start <= r.end && r.end <= count);
      assert.ok(r.top >= 0 && r.bottom >= 0);
      const renderedRows = Math.ceil((r.end - r.start) / columns);
      assert.equal(r.top + (renderedRows ? renderedRows * stride - gap : 0) + r.bottom, totalHeight);
      assert.ok(r.end - r.start <= (Math.ceil(height / stride) + 1 + 2 * overscan) * columns);
      if (r.visible) {
        const localTop = Math.max(0, scrollTop - offset);
        const localBottom = Math.min(totalHeight, scrollTop + height - offset);
        assert.ok(r.start <= Math.floor(localTop / stride) * columns);
        assert.ok(r.end >= Math.min(count, Math.ceil(localBottom / stride) * columns));
      } else {
        assert.equal(r.end - r.start, 0);
      }
    }
  }
});
