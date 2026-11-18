import test from 'node:test';
import assert from 'node:assert/strict';
import { paintClock } from './flip-clock.js';

// Exercise the public painter with persistent nodes and real event ordering.
function fixture() {
  let reduced = false, created = 0;
  class Element {
    constructor() { this.children = []; this.dataset = {}; this.attributes = {}; this.className = ''; this.listeners = new Map(); this._text = ''; }
    get classList() { return { contains: name => this.className.split(' ').includes(name), add: name => { if (!this.classList.contains(name)) this.className += ` ${name}`; }, remove: name => { this.className = this.className.split(' ').filter(value => value !== name).join(' '); } }; }
    get textContent() { return this._text + this.children.map(child => child.textContent).join(''); }
    set textContent(value) { this.children.forEach(child => { child.parent = null; }); this.children = []; this._text = value; }
    setAttribute(name, value) { this.attributes[name] = value; }
    append(...nodes) { for (const node of nodes) { node.remove(); node.parent = this; this.children.push(node); } }
    replaceChildren(...nodes) { this.textContent = ''; this.append(...nodes); }
    remove() { if (this.parent) { this.parent.children = this.parent.children.filter(child => child !== this); this.parent = null; } }
    querySelectorAll(selector) { return this.children.flatMap(child => [...(child.classList.contains(selector.slice(1)) ? [child] : []), ...child.querySelectorAll(selector)]); }
    querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
    addEventListener(type, listener, options) { const handlers = this.listeners.get(type) || []; handlers.push({ listener, once: options?.once }); this.listeners.set(type, handlers); }
    dispatch(type) { const handlers = this.listeners.get(type) || []; this.listeners.set(type, handlers.filter(handler => !handler.once)); handlers.forEach(handler => handler.listener({ type, target: this })); }
  }
  const document = { defaultView: { matchMedia: () => ({ matches: reduced }) }, createElement() { created++; return new Element(); } };
  const element = new Element(); element.ownerDocument = document;
  return { element, created: () => created, reduce: value => { reduced = value; } };
}

test('fresh and virtualized clocks mount every digit directly without invented flips', () => {
  for (const value of ['00:00:09', '123:45:06']) {
    const f = fixture(); paintClock(f.element, value);
    assert.equal(f.element.querySelectorAll('.clock-digit').length, value.length - 2);
    assert.equal(f.element.querySelectorAll('.clock-colon').length, 2);
    assert.equal(f.element.textContent, value);
    assert.equal(f.element.attributes['aria-label'], `累计监控时长 ${value}`);
    assert.equal(f.element.querySelectorAll('.clock-face').length, 0);
    assert.ok(f.element.children.every(cell => cell.attributes['aria-hidden'] === 'true'));
  }
});

test('a carry flips only its changed digits and keeps every existing digit node', () => {
  const f = fixture(); paintClock(f.element, '00:00:09');
  const cells = [...f.element.children];
  paintClock(f.element, '00:00:10');
  assert.deepEqual(f.element.children, cells);
  assert.deepEqual(cells.map((cell, index) => cell.classList.contains('is-flipping') ? index : null).filter(index => index !== null), [6, 7]);
  assert.equal(cells[7].querySelector('.clock-previous').textContent, '9');
  assert.equal(cells[7].querySelector('.clock-out').textContent, '9');
  assert.equal(cells[7].querySelector('.clock-in').textContent, '0');
  for (const cell of cells.slice(6)) cell.querySelector('.clock-in').dispatch('animationend');
  assert.equal(f.element.textContent, '00:00:10');
  assert.equal(f.element.querySelectorAll('.clock-face').length, 0);
  assert.equal(f.element.querySelectorAll('.is-flipping').length, 0);
});

test('unchanged updates allocate no new nodes and do not restart a flip', () => {
  const f = fixture(); paintClock(f.element, '00:00:09'); paintClock(f.element, '00:00:10');
  const count = f.created(), incoming = f.element.querySelector('.clock-in');
  for (let i = 0; i < 100; i++) paintClock(f.element, '00:00:10');
  assert.equal(f.created(), count);
  assert.equal(f.element.querySelector('.clock-in'), incoming);
});

test('an interrupted old animation cannot remove a newer digit animation', () => {
  const f = fixture(); paintClock(f.element, '00:00:01'); paintClock(f.element, '00:00:02');
  const old = f.element.querySelector('.clock-in');
  paintClock(f.element, '00:00:03');
  const current = f.element.querySelector('.clock-in');
  old.dispatch('animationcancel');
  assert.equal(f.element.querySelector('.clock-in'), current);
  assert.equal(f.element.querySelectorAll('.clock-face').length, 3);
  assert.equal(f.element.querySelectorAll('.is-flipping').length, 1);
  current.dispatch('animationend');
  assert.equal(f.element.textContent, '00:00:03');
  assert.equal(f.element.querySelectorAll('.clock-face').length, 0);
});

test('reduced motion cancels an in-flight fold even when the time value is unchanged', () => {
  const f = fixture(); paintClock(f.element, '00:00:09'); paintClock(f.element, '00:00:10');
  const count = f.created(); f.reduce(true); paintClock(f.element, '00:00:10');
  assert.equal(f.created(), count);
  assert.equal(f.element.textContent, '00:00:10');
  assert.equal(f.element.querySelectorAll('.is-flipping').length, 0);
  assert.equal(f.element.querySelectorAll('.clock-face').length, 0);
  paintClock(f.element, '00:00:11');
  assert.equal(f.element.textContent, '00:00:11');
  assert.equal(f.element.querySelectorAll('.clock-face').length, 0);
});

test('unknown values and increasing hour widths rebuild directly without stale halves', () => {
  const f = fixture(); paintClock(f.element, '99:59:59'); paintClock(f.element, '100:00:00');
  assert.equal(f.element.textContent, '100:00:00');
  assert.equal(f.element.querySelectorAll('.clock-digit').length, 7);
  assert.equal(f.element.querySelectorAll('.clock-face').length, 0);
  paintClock(f.element, '—'); assert.equal(f.element.textContent, '—');
  paintClock(f.element, '00:00:01');
  assert.equal(f.element.textContent, '00:00:01');
  assert.equal(f.element.querySelectorAll('.clock-face').length, 0);
});
