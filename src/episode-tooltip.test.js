import test from "node:test";
import assert from "node:assert/strict";
import { createEpisodeTooltip, episodeTooltipPosition } from "./episode-tooltip.js";

test("episode statistics choose the available side and stay inside narrow viewports", () => {
  assert.deepEqual(episodeTooltipPosition({ left: 900, top: 500, bottom: 540 }, { width: 284, height: 300 }, { width: 1040, height: 700 }), {
    left: 744, top: 192, maxWidth: 1016, maxHeight: 676, placement: "above",
  });
  const below = episodeTooltipPosition({ left: 100, top: 20, bottom: 60 }, { width: 284, height: 300 }, { width: 320, height: 700 });
  assert.equal(below.placement, "below");
  assert.equal(below.top, 68);
  assert.equal(below.left, 24);
  const small = episodeTooltipPosition({ left: 170, top: 80, bottom: 110 }, { width: 284, height: 300 }, { width: 200, height: 160 });
  assert.equal(small.left, 12);
  assert.equal(small.top, 12);
  assert.equal(small.maxWidth, 176);
  assert.equal(small.maxHeight, 136);
});

function fixture() {
  const sources = new Map();
  const owner = { isConnected: true, querySelectorAll: () => [...sources.values()] };
  const makeCard = (session, content) => {
    const source = { innerHTML: content };
    const card = { isConnected: true, dataset: { session }, closest: selector => selector === ".quality-episode" ? card : owner,
      querySelector: () => source, getBoundingClientRect: () => ({ left: 80, top: 500, bottom: 540 }) };
    sources.set(session, card);
    return card;
  };
  const portals = [];
  const listeners = new Map();
  const view = { innerWidth: 1040, innerHeight: 700,
    addEventListener: (name, handler) => listeners.set(name, handler), removeEventListener: name => listeners.delete(name) };
  const doc = { body: { append: portal => portals.push(portal) }, createElement: () => ({
    style: {}, dataset: {}, attributes: {}, setAttribute(name, value) { this.attributes[name] = value; },
    querySelectorAll: () => [], getBoundingClientRect: () => ({ width: 284, height: 300 }),
    remove() { this.removed = true; },
  }) };
  return { controller: createEpisodeTooltip({ document: doc, window: view }), portals, listeners, makeCard };
}

test("floating statistics refresh the same session after cards are replaced and dismiss on scroll", () => {
  const f = fixture();
  const first = f.makeCard("101", "原统计");
  assert.equal(f.controller.show(first), true);
  assert.equal(f.portals[0].attributes["aria-hidden"], "true");
  assert.equal(f.portals[0].style.position, "fixed");
  assert.equal(f.portals[0].style.top, "192px");
  first.isConnected = false;
  f.makeCard("101", "新统计");
  assert.equal(f.controller.refresh(), true);
  assert.equal(f.portals[0].innerHTML, "新统计");
  f.listeners.get("scroll")();
  assert.equal(f.portals[0].removed, true);
  assert.equal(f.controller.refresh(), false);
  f.controller.destroy();
  assert.equal(f.listeners.size, 0);
  assert.equal(f.controller.show(first), false);
});
test("short dialogs place the card beside its trigger without covering header controls",()=>{
 const pos=episodeTooltipPosition({left:513,right:729,top:277,bottom:345},{width:268,height:220},{width:804,height:623,top:105,bottom:518});
 assert.equal(pos.placement,"left");assert.equal(pos.left,237);assert.ok(pos.top>=105);assert.ok(pos.top+220<=518);
});
