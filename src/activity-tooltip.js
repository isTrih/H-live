import {episodeTooltipPosition} from "./episode-tooltip.js";

export function createActivityTooltip({document: doc = globalThis.document, window: view = globalThis.window} = {}) {
  const selector = ".timeline-activity-marker, .timeline-activity-interval";
  let card = null, owner = null, portal = null, activityId = "", interval = false, scope = "", destroyed = false;
  const trigger = target => target?.closest?.(selector) || null;
  const trackScope = target => target?.closest?.(".timeline-integrated-progress") ? "media" : "archive";

  function hide() {
    portal?.remove();
    portal = null;
    card = null;
    owner = null;
    activityId = "";
  }
  function paint() {
    const source = card?.querySelector(".timeline-activity-tooltip");
    const host = doc?.fullscreenElement || doc?.body;
    if (!source || !host || card.isConnected === false || owner?.isConnected === false) { hide(); return false; }
    if (!portal) {
      portal = doc.createElement("div");
      portal.className = "timeline-activity-tooltip activity-floating-tooltip";
      portal.setAttribute("role", "tooltip");
      portal.setAttribute("aria-hidden", "true");
      Object.assign(portal.style, {display: "flex", position: "fixed", bottom: "auto", right: "auto", margin: "0", boxSizing: "border-box", pointerEvents: "none", zIndex: "999", opacity: "1", visibility: "visible", transform: "none"});
    }
    if (portal.parentElement !== host) host.append(portal);
    portal.classList.toggle("red_packet", card.classList.contains("red_packet"));
    portal.classList.toggle("activity-density", card.classList.contains("activity-density"));
    portal.innerHTML = source.innerHTML;
    for (const element of portal.querySelectorAll("[id]")) element.removeAttribute("id");
    const viewport = {width: view?.innerWidth || doc.documentElement?.clientWidth || 0, height: view?.innerHeight || doc.documentElement?.clientHeight || 0};
    const body = !doc.fullscreenElement && owner?.closest?.(".modal-body")?.getBoundingClientRect?.();
    if (body) { viewport.top = body.top + 8; viewport.bottom = body.bottom - 8; }
    portal.style.maxWidth = `${Math.max(0, viewport.width - 24)}px`;
    portal.style.maxHeight = `${Math.max(0, body ? viewport.bottom - viewport.top : viewport.height - 24)}px`;
    const position = episodeTooltipPosition(card.getBoundingClientRect(), portal.getBoundingClientRect(), viewport);
    portal.style.left = `${position.left}px`;
    portal.style.top = `${position.top}px`;
    portal.dataset.placement = position.placement;
    return true;
  }
  function show(target) {
    if (destroyed) return false;
    const next = trigger(target);
    if (!next) { hide(); return false; }
    if (next === card && portal?.isConnected) return true;
    card = next;
    owner = card.closest("#session-timeline") || card.closest(".timeline-shell") || doc;
    activityId = card.dataset.activityKey || card.dataset.activity || "";
    interval = card.classList.contains("timeline-activity-interval");
    scope = trackScope(card);
    return paint();
  }
  function refresh() {
    if (destroyed || !card) return false;
    if (card.isConnected === false && owner?.isConnected !== false) {
      const candidates = [...(owner.querySelectorAll?.(selector) || [])];
      card = candidates.find(item => (item.dataset.activityKey || item.dataset.activity || "") === activityId && item.classList.contains("timeline-activity-interval") === interval && trackScope(item) === scope) || null;
    }
    if (!card) { hide(); return false; }
    return paint();
  }
  function hover(event) {
    const next = trigger(event.target);
    if (next) show(next);
    else if (!trigger(doc.activeElement)) hide();
  }
  function leave(event) {
    if (!card || card.contains(event.relatedTarget)) return;
    const next = trigger(event.relatedTarget) || trigger(doc.activeElement);
    if (next) show(next); else hide();
  }
  function focus(event) { show(event.target); }
  function blur(event) { const next = trigger(event.relatedTarget); if (next) show(next); else hide(); }
  const fullScreen = () => { if (card) paint(); };
  const listeners = [["mouseover", hover], ["mousemove", hover], ["mouseout", leave], ["focusin", focus], ["focusout", blur], ["fullscreenchange", fullScreen]];
  for (const [name, handler] of listeners) doc?.addEventListener?.(name, handler);
  view?.addEventListener?.("scroll", hide, {capture: true, passive: true});
  view?.addEventListener?.("resize", fullScreen, {passive: true});
  function destroy() {
    destroyed = true;
    hide();
    for (const [name, handler] of listeners) doc?.removeEventListener?.(name, handler);
    view?.removeEventListener?.("scroll", hide, {capture: true});
    view?.removeEventListener?.("resize", fullScreen);
  }
  return {show, hide, refresh, destroy};
}
