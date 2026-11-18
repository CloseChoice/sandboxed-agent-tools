(() => {
  const elements = [], text = [], max = 200;
  let visited = 0, truncated = false;
  function visit(root, prefix = '') {
    for (const el of root.children) {
      if (++visited > 5000 || elements.length >= max) { truncated = true; return; }
      const tag = el.tagName.toLowerCase();
      if (['script', 'style', 'noscript', 'template'].includes(tag)) continue;
      const r = el.getBoundingClientRect(), s = getComputedStyle(el);
      const visible = r.width > 0 && r.height > 0 && s.display !== 'none' && s.visibility !== 'hidden';
      if (visible && el.matches('button,input,textarea,select,a[href],[role],[contenteditable],h1,h2,h3,label')) {
        const selector = prefix + (el.id ? '#' + CSS.escape(el.id) : path(el));
        elements.push({selector, tag, role: el.getAttribute('role'),
          name: (el.getAttribute('aria-label') || el.innerText || el.getAttribute('placeholder') || '').slice(0,300),
          value: tag === 'input' && el.type === 'password' ? '[redacted]' : (el.value == null ? null : String(el.value).slice(0,1000)),
          disabled: !!el.disabled});
      }
      if (visible && el.children.length === 0 && el.textContent.trim()) text.push(el.textContent.trim().slice(0,500));
      if (el.shadowRoot) visit(el.shadowRoot, prefix + (el.id ? '#' + CSS.escape(el.id) : path(el)) + ' >>> ');
      if (el.contentDocument) visit(el.contentDocument, prefix + (el.id ? '#' + CSS.escape(el.id) : path(el)) + ' >>> ');
      visit(el, prefix);
    }
  }
  function path(el) {
    const parts = [];
    while (el && el.nodeType === 1) {
      if (el.id) { parts.unshift('#' + CSS.escape(el.id)); break; }
      const tag = el.tagName.toLowerCase();
      const siblings = el.parentNode ? [...el.parentNode.children].filter(e => e.tagName === el.tagName) : [];
      parts.unshift(tag + (siblings.length > 1 ? ':nth-of-type(' + (siblings.indexOf(el)+1) + ')' : ''));
      el = el.parentElement;
    }
    return parts.join(' > ');
  }
  visit(document);
  return {url: location.href, title: document.title, elements, text: text.join('\n').slice(0,12000), truncated};
})()
