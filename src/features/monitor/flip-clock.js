// Update only changing digits. A freshly mounted/virtualized clock never flips
// through a made-up starting time, and no separate timer runs for each card.
export function paintClock(element, value) {
  const previous = element.dataset.time;
  const reduced = element.ownerDocument.defaultView.matchMedia('(prefers-reduced-motion: reduce)').matches;
  if (reduced) {
    element.querySelectorAll('.is-flipping').forEach(cell => {
      cell.querySelectorAll('.clock-face').forEach(face => face.remove());
      cell.classList.remove('is-flipping');
    });
  }
  if (previous === value) return;
  element.dataset.time = value;
  element.setAttribute('aria-label', `累计监控时长 ${value}`);
  const valid = /^\d{2,}:\d{2}:\d{2}$/;
  const document = element.ownerDocument;
  const span = (cls, text) => {
    const node = document.createElement('span');
    node.className = cls;
    node.textContent = text;
    return node;
  };
  if (!valid.test(value)) {
    element.textContent = value;
    return;
  }
  if (!valid.test(previous || '') || previous.length !== value.length) {
    element.replaceChildren(...Array.from(value, char => {
      const cell = span(char === ':' ? 'clock-colon' : 'clock-digit', '');
      cell.setAttribute('aria-hidden', 'true');
      cell.append(span('clock-value', char));
      return cell;
    }));
    return;
  }
  Array.from(element.children).forEach((cell, i) => {
    if (value[i] === previous[i]) return;
    cell.querySelectorAll('.clock-face').forEach(face => face.remove());
    cell.classList.remove('is-flipping');
    cell.querySelector('.clock-value').textContent = value[i];
    if (reduced) return;
    const old = span('clock-face clock-out', previous[i]);
    const next = span('clock-face clock-in', value[i]);
    cell.append(old, next);
    cell.classList.add('is-flipping');
    next.addEventListener('animationend', () => {
      old.remove();
      next.remove();
      if (!cell.querySelector('.clock-face')) cell.classList.remove('is-flipping');
    }, {once:true});
  });
}
