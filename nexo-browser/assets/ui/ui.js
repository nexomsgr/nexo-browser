(() => {
  'use strict';

  const SVG = (inner, size = 18) =>
    '<svg viewBox="0 0 24 24" width="' + size + '" height="' + size + '" fill="none" stroke="currentColor" ' +
    'stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + inner + '</svg>';

  // Иконки — статические строки (данных от сайтов в них нет).
  const ICON = {
    back: SVG('<path d="M15 6l-6 6 6 6"/>'),
    forward: SVG('<path d="M9 6l6 6-6 6"/>'),
    reload: SVG('<path d="M20 12a8 8 0 1 1-2.4-5.7"/><path d="M20 4v5h-5"/>'),
    stop: SVG('<path d="M6 6l12 12M18 6L6 18"/>'),
    lock: SVG('<rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>'),
    warn: SVG('<circle cx="12" cy="12" r="9"/><path d="M12 7.5v5M12 16v.5"/>'),
    search: SVG('<circle cx="11" cy="11" r="7"/><path d="M20 20l-4-4"/>'),
    plus: SVG('<path d="M12 5v14M5 12h14"/>'),
    star: SVG('<path d="M12 3.5l2.7 5.6 6.1.8-4.5 4.2 1.1 6-5.4-3-5.4 3 1.1-6L3.2 9.9l6.1-.8z"/>'),
    gear: SVG('<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1"/>'),
    starOn: SVG('<path fill="currentColor" d="M12 3.5l2.7 5.6 6.1.8-4.5 4.2 1.1 6-5.4-3-5.4 3 1.1-6L3.2 9.9l6.1-.8z"/>'),
    close: SVG('<path d="M6 6l12 12M18 6L6 18"/>', 12),
  };

  const $ = (id) => document.getElementById(id);
  const app = $('app');
  const tabsEl = $('tabs');
  const newTabBtn = $('new-tab');
  const backBtn = $('back');
  const fwdBtn = $('forward');
  const reloadBtn = $('reload');
  const form = $('address-form');
  const input = $('url');
  const sec = $('sec');
  const starBtn = $('star');
  const menuBtn = $('menu');

  let state = { tabs: [], address: '', security: 'none', can_back: false, can_forward: false, loading: false, bookmarked: false, can_bookmark: false };
  let drag = null;          // активное перетаскивание вкладки
  let pending = false;      // состояние пришло во время перетаскивания — применим после

  const send = (msg) => window.ipc.postMessage(JSON.stringify(msg));
  const activeTab = () => state.tabs.find((t) => t.active);

  function setIcon(el, svg) {
    if (el.dataset.icon !== svg) { el.innerHTML = svg; el.dataset.icon = svg; }
  }

  // ---------- Вкладки: элементы переиспользуются по id, чтобы не терять pointer capture при перетаскивании ----------
  const tabEls = new Map();

  function createTab(id) {
    const el = document.createElement('div');
    el.className = 'tab enter';
    el.dataset.id = String(id);
    el.setAttribute('role', 'tab');
    const dot = document.createElement('span');
    dot.className = 'dot';
    const title = document.createElement('span');
    title.className = 'title';
    const close = document.createElement('button');
    close.className = 'close';
    close.title = 'Закрыть вкладку';
    close.setAttribute('aria-label', 'Закрыть вкладку');
    close.innerHTML = ICON.close;
    el.append(dot, title, close);
    el.addEventListener('animationend', () => el.classList.remove('enter'), { once: true });
    return el;
  }

  function updateTab(el, t) {
    el.classList.toggle('active', t.active);
    el.classList.toggle('loading', t.loading);
    el.classList.toggle('sleeping', t.sleeping);
    el.setAttribute('aria-selected', String(t.active));
    const title = el.children[1];
    if (title.textContent !== t.title) title.textContent = t.title;   // только textContent: заголовок задаёт сайт
    el.title = t.sleeping ? t.title + ' — вкладка спит' : t.title;
  }

  function renderTabs() {
    let cursor = tabsEl.firstChild;
    const live = new Set();
    for (const t of state.tabs) {
      let el = tabEls.get(t.id);
      if (!el) { el = createTab(t.id); tabEls.set(t.id, el); }
      updateTab(el, t);
      live.add(t.id);
      if (el === cursor) cursor = cursor.nextSibling;
      else tabsEl.insertBefore(el, cursor);
    }
    for (const [id, el] of tabEls) {
      if (!live.has(id)) { el.remove(); tabEls.delete(id); }
    }
    revealActive();
  }

  // Если вкладок много и лента прокручивается — активная вкладка всегда должна быть видна.
  function revealActive() {
    const cur = activeTab();
    const el = cur && tabEls.get(cur.id);
    if (!el || tabsEl.scrollWidth <= tabsEl.clientWidth) return;
    const left = el.offsetLeft - tabsEl.offsetLeft;
    const right = left + el.offsetWidth;
    if (left < tabsEl.scrollLeft) tabsEl.scrollLeft = left;
    else if (right > tabsEl.scrollLeft + tabsEl.clientWidth) tabsEl.scrollLeft = right - tabsEl.clientWidth;
  }

  // Колесо мыши над лентой вкладок прокручивает её по горизонтали.
  tabsEl.addEventListener('wheel', (e) => {
    if (e.deltaY !== 0) { tabsEl.scrollLeft += e.deltaY; e.preventDefault(); }
  }, { passive: false });

  function render() {
    const cur = activeTab();
    const loading = !!(cur && cur.loading);
    app.classList.toggle('loading', loading);
    backBtn.disabled = !state.can_back;
    fwdBtn.disabled = !state.can_forward;
    setIcon(backBtn, ICON.back);
    setIcon(fwdBtn, ICON.forward);
    setIcon(newTabBtn, ICON.plus);
    setIcon(menuBtn, ICON.gear);
    setIcon(reloadBtn, loading ? ICON.stop : ICON.reload);
    reloadBtn.title = loading ? 'Остановить' : 'Обновить';
    reloadBtn.setAttribute('aria-label', reloadBtn.title);

    sec.className = 'sec ' + state.security;
    setIcon(sec, state.security === 'secure' ? ICON.lock
      : state.security === 'insecure' ? ICON.warn : ICON.search);
    sec.title = state.security === 'secure' ? 'Соединение защищено'
      : state.security === 'insecure' ? 'Соединение не защищено' : '';

    starBtn.hidden = !state.can_bookmark;
    starBtn.classList.toggle('on', !!state.bookmarked);
    setIcon(starBtn, state.bookmarked ? ICON.starOn : ICON.star);
    starBtn.title = state.bookmarked ? 'Убрать из закладок (Ctrl+D)' : 'В закладки (Ctrl+D)';
    if (document.activeElement !== input) input.value = state.address;
    renderTabs();
  }

  window.nexo = {
    setState(s) {
      state = s;
      if (drag && drag.moved) { pending = true; return; }
      render();
    },
    focusAddress() { input.focus(); input.select(); },
  };

  menuBtn.addEventListener('click', () => send({ cmd: 'open_internal', page: 'settings' }));
  starBtn.addEventListener('click', () => send({ cmd: 'toggle_bookmark' }));

  // ---------- Кнопки ----------
  backBtn.addEventListener('click', () => send({ cmd: 'back' }));
  fwdBtn.addEventListener('click', () => send({ cmd: 'forward' }));
  newTabBtn.addEventListener('click', () => send({ cmd: 'new_tab' }));
  reloadBtn.addEventListener('click', () => {
    const cur = activeTab();
    send({ cmd: cur && cur.loading ? 'stop' : 'reload' });
  });

  form.addEventListener('submit', (e) => {
    e.preventDefault();
    send({ cmd: 'navigate', input: input.value });
    input.blur();
  });
  input.addEventListener('focus', () => input.select());
  input.addEventListener('blur', () => { input.value = state.address; });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') { input.value = state.address; input.blur(); }
  });

  // ---------- Клики по вкладкам, закрытие, перетаскивание ----------
  const tabIdOf = (target) => {
    const el = target.closest && target.closest('.tab');
    return el ? Number(el.dataset.id) : null;
  };

  tabsEl.addEventListener('click', (e) => {
    if (e.target.closest('.close')) {
      const id = tabIdOf(e.target);
      if (id !== null) send({ cmd: 'close_tab', id });
    }
  });

  // Средняя кнопка мыши закрывает вкладку.
  tabsEl.addEventListener('auxclick', (e) => {
    if (e.button !== 1) return;
    const id = tabIdOf(e.target);
    if (id !== null) { e.preventDefault(); send({ cmd: 'close_tab', id }); }
  });
  tabsEl.addEventListener('mousedown', (e) => { if (e.button === 1) e.preventDefault(); });

  tabsEl.addEventListener('pointerdown', (e) => {
    if (e.button !== 0 || e.target.closest('.close')) return;
    const id = tabIdOf(e.target);
    if (id === null) return;
    const t = state.tabs.find((x) => x.id === id);
    if (t && !t.active) send({ cmd: 'activate_tab', id });
    const el = tabEls.get(id);
    drag = { id, el, startX: e.clientX, moved: false, index: state.tabs.findIndex((x) => x.id === id) };
    el.setPointerCapture(e.pointerId);
  });

  tabsEl.addEventListener('pointermove', (e) => {
    if (!drag) return;
    const dx = e.clientX - drag.startX;
    if (!drag.moved && Math.abs(dx) < 5) return;
    drag.moved = true;
    drag.el.classList.add('dragging');
    drag.el.style.transform = 'translateX(' + dx + 'px)';
    // Новая позиция: сколько других вкладок целиком левее курсора.
    let idx = 0;
    for (const el of tabsEl.children) {
      if (el === drag.el) continue;
      const r = el.getBoundingClientRect();
      if (e.clientX > r.left + r.width / 2) idx++;
    }
    drag.index = idx;
  });

  function endDrag() {
    if (!drag) return;
    const d = drag;
    drag = null;
    if (d.moved) {
      d.el.classList.remove('dragging');
      d.el.style.transform = '';
      const from = state.tabs.findIndex((x) => x.id === d.id);
      if (d.index !== from) send({ cmd: 'move_tab', id: d.id, index: d.index });
    }
    if (pending) { pending = false; render(); }
  }
  tabsEl.addEventListener('pointerup', endDrag);
  tabsEl.addEventListener('pointercancel', endDrag);

  // ---------- Горячие клавиши, когда фокус в панели интерфейса ----------
  // Таблица клавиш живёт в Rust (shortcuts.rs) и общая для панели и страниц сайтов; здесь только пересылка.
  // Обычный ввод и стандартные правки текста (Ctrl+A/C/V/X/Z/Y) в адресной строке не перехватываем.
  const EDIT_KEYS = new Set(['a', 'c', 'v', 'x', 'z', 'y']);
  document.addEventListener('keydown', (e) => {
    const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    const fkey = /^F([1-9]|1[0-2])$/.test(k);
    if (!(e.ctrlKey || e.altKey || fkey)) return;
    if (e.ctrlKey && !e.altKey && EDIT_KEYS.has(k)) return;
    e.preventDefault();
    send({ cmd: 'key', ctrl: e.ctrlKey, shift: e.shiftKey, alt: e.altKey, key: e.key });
  });

  render();
})();
