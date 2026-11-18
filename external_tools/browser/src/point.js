(() => {
  const el = SAT_SELECTOR;
  if (el.disabled || el.getAttribute('aria-disabled') === 'true') throw Error('element is disabled');
  el.scrollIntoView({block:'center', inline:'center', behavior:'instant'});
  const r = el.getBoundingClientRect(), view = el.ownerDocument.defaultView;
  let x = (Math.min(view.innerWidth, r.right) + Math.max(0, r.left)) / 2;
  let y = (Math.min(view.innerHeight, r.bottom) + Math.max(0, r.top)) / 2;
  let hit = el.getRootNode().elementFromPoint(x, y);
  if (hit !== el && !el.contains(hit)) throw Error('element is covered');
  let frame = view.frameElement;
  while (frame) {
    const rect = frame.getBoundingClientRect();
    x += rect.left + frame.clientLeft;
    y += rect.top + frame.clientTop;
    hit = frame.getRootNode().elementFromPoint(x, y);
    if (hit !== frame) throw Error('frame is covered');
    frame = frame.ownerDocument.defaultView.frameElement;
  }
  return {x, y};
})()
