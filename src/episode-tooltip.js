const margin = 12;
const number = value => Number.isFinite(value) ? value : 0;
const clamp = (value, min, max) => Math.min(Math.max(min, max), Math.max(min, value));

// Keep the statistics visible even when a card is close to a viewport edge.
export function episodeTooltipPosition(rect, size, viewport, gap = 8) {
  const viewportWidth = Math.max(0, number(viewport.width));
  const viewportHeight = Math.max(0, number(viewport.height));
  const insetX = Math.min(margin, viewportWidth / 2);
  const insetY = Math.max(Math.min(margin, viewportHeight / 2),number(viewport.top));
  const bottomEdge=Math.min(viewportHeight-Math.min(margin,viewportHeight/2),Number.isFinite(viewport.bottom)?viewport.bottom:viewportHeight);
  const maxWidth = Math.max(0, viewportWidth - insetX * 2);
  const maxHeight = Math.max(0, bottomEdge-insetY);
  const width = Math.min(Math.max(0, number(size.width)), maxWidth);
  const height = Math.min(Math.max(0, number(size.height)), maxHeight);
  const top = number(rect.top);
  const bottom = number(rect.bottom);
  const above = top - gap - insetY;
  const below = bottomEdge-bottom-gap;
  // Use the side when neither vertical gap fits, preserving the trigger and
  // the dialog's header/footer controls in short desktop viewports.
  const right=Number.isFinite(rect.right)?rect.right:number(rect.left);
  if(above<height&&below<height){
    const onRight=viewportWidth-insetX-right-gap>=width;
    const onLeft=number(rect.left)-gap-insetX>=width;
    if(onRight||onLeft)return {left:onRight?right+gap:number(rect.left)-gap-width,top:clamp((top+bottom-height)/2,insetY,bottomEdge-height),maxWidth,maxHeight,placement:onRight?"right":"left"};
  }
  const placement = above >= height || (below < height && above > below) ? "above" : "below";
  return {
    left: clamp(number(rect.left), insetX, viewportWidth - insetX - width),
    top: clamp(placement === "above" ? top - gap - height : bottom + gap, insetY, bottomEdge-height),
    maxWidth, maxHeight, placement,
  };
}

export function createEpisodeTooltip({ document: doc = globalThis.document, window: view = globalThis.window } = {}) {
  let card = null;
  let owner = null;
  let sessionId = "";
  let portal = null;
  let destroyed = false;

  function hide() {
    portal?.remove();
    portal = null;
    card = null;
    owner = null;
    sessionId = "";
  }

  function viewport() {
    return {
      width: view?.innerWidth || doc?.documentElement?.clientWidth || 0,
      height: view?.innerHeight || doc?.documentElement?.clientHeight || 0,
    };
  }

  function paint() {
    const source = card?.querySelector(".episode-tooltip");
    if (!source || card.isConnected === false || owner?.isConnected === false || !doc?.body) { hide(); return false; }
    if (!portal) {
      portal = doc.createElement("div");
      portal.className = "episode-tooltip episode-floating-tooltip";
      portal.setAttribute("role", "tooltip");
      // The original card's aria-describedby remains the single AT description.
      portal.setAttribute("aria-hidden", "true");
      Object.assign(portal.style, {
        display: "block", position: "fixed", bottom: "auto", right: "auto", margin: "0",
        boxSizing: "border-box", pointerEvents: "none", zIndex: "90", overflowY: "auto",
      });
      doc.body.append(portal);
    }
    portal.innerHTML = source.innerHTML;
    for (const element of portal.querySelectorAll("[id]")) element.removeAttribute("id");
    const bounds = viewport();
    const dialogBody=owner?.closest?.(".modal-body")?.getBoundingClientRect?.();
    if(dialogBody){bounds.top=dialogBody.top+8;bounds.bottom=dialogBody.bottom-8;}
    portal.style.maxWidth = `${Math.max(0, bounds.width - margin * 2)}px`;
    portal.style.maxHeight = `${Math.max(0,Math.min(bounds.height-margin*2,dialogBody?bounds.bottom-bounds.top:bounds.height))}px`;
    const position = episodeTooltipPosition(card.getBoundingClientRect(), portal.getBoundingClientRect(), bounds);
    portal.style.left = `${position.left}px`;
    portal.style.top = `${position.top}px`;
    portal.dataset.placement = position.placement;
    return true;
  }

  function show(target) {
    if (destroyed) return false;
    const next = target?.closest?.(".quality-episode");
    if (!next) { hide(); return false; }
    card = next;
    owner = card.closest("#session-quality") || card.closest(".anchor-detail-dialog") || doc;
    sessionId = card.dataset.session || "";
    return paint();
  }

  function refresh() {
    if (destroyed || !card) return false;
    if (card.isConnected === false && owner?.isConnected !== false && sessionId) {
      card = [...(owner?.querySelectorAll(".quality-episode") || [])].find(candidate => candidate.dataset.session === sessionId) || null;
    }
    if (!card) { hide(); return false; }
    return paint();
  }

  // Scrolling any ancestor changes the card's screen position. Dismiss rather
  // than leaving a detached statistics panel over unrelated content.
  view?.addEventListener("scroll", hide, { capture: true, passive: true });
  view?.addEventListener("resize", hide, { passive: true });
  function destroy() {
    hide();
    destroyed = true;
    view?.removeEventListener("scroll", hide, true);
    view?.removeEventListener("resize", hide);
  }

  return { show, hide, refresh, destroy };
}
