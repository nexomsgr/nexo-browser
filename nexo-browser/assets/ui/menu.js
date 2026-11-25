(() => {
  'use strict';
  const send = (m) => window.ipc.postMessage(JSON.stringify(m));
  const svg = (p) => '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + p + '</svg>';
  const IC = {
    tab: svg('<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M12 9v6M9 12h6"/>'),
    priv: svg('<path d="M4 11c0-5 3-7 8-7s8 2 8 7v2H4z"/><circle cx="8.5" cy="17" r="2.5"/><circle cx="15.5" cy="17" r="2.5"/>'),
    hist: svg('<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>'),
    dl: svg('<path d="M12 4v11M7 11l5 5 5-5M5 20h14"/>'),
    bm: svg('<path d="M7 4h10v16l-5-4-5 4z"/>'),
    key: svg('<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M16 7l3 3M14 9l2 2"/>'),
    perf: svg('<path d="M4 18a8 8 0 1 1 16 0"/><path d="M12 18l4-6"/>'),
    mem: svg('<rect x="4" y="8" width="16" height="8" rx="1.5"/><path d="M8 8V5M12 8V5M16 8V5M8 19v-3M12 19v-3M16 19v-3"/>'),
    print: svg('<path d="M7 9V4h10v5M7 17H5v-6h14v6h-2"/><rect x="7" y="14" width="10" height="6"/>'),
    full: svg('<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>'),
    gear: svg('<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1"/>'),
    exit: svg('<path d="M10 4H5v16h5M15 8l4 4-4 4M19 12H9"/>'),
  };
  const ITEMS = [
    ['new_tab', IC.tab, 'Новая вкладка', 'Ctrl+T'],
    ['private', IC.priv, 'Приватная вкладка', 'Ctrl+Shift+N'],
    '-',
    ['bookmarks', IC.bm, 'Закладки', 'Ctrl+Shift+O'],
    ['history', IC.hist, 'История', 'Ctrl+H'],
    ['downloads', IC.dl, 'Загрузки', 'Ctrl+J'],
    ['passwords', IC.key, 'Пароли', ''],
    '-',
    'zoom',
    '-',
    ['print', IC.print, 'Печать…', 'Ctrl+P'],
    ['fullscreen', IC.full, 'Полноэкранный режим', 'F11'],
    '-',
    ['performance', IC.perf, 'Производительность', ''],
    ['free_memory', IC.mem, 'Освободить память', '', 'amber'],
    ['settings', IC.gear, 'Настройки', ''],
    '-',
    ['exit', IC.exit, 'Выход', ''],
  ];
  const root = document.getElementById('menu');
  let zv = null;

  const el = (tag, cls) => { const e = document.createElement(tag); if (cls) e.className = cls; return e; };
  for (const it of ITEMS) {
    if (it === '-') { root.append(el('div', 'sep')); continue; }
    if (it === 'zoom') {
      const z = el('div', 'zoom');
      const l = el('span', 'label'); l.textContent = 'Масштаб';
      const mk = (t, id, title) => { const b = el('button'); b.textContent = t; b.title = title; b.addEventListener('click', () => send({ cmd: 'menu_action', id })); return b; };
      zv = el('span'); zv.id = 'zv'; zv.textContent = '100%';
      zv.addEventListener('dblclick', () => send({ cmd: 'menu_action', id: 'zoom_reset' }));
      z.append(l, mk('−', 'zoom_out', 'Уменьшить'), zv, mk('+', 'zoom_in', 'Увеличить'));
      root.append(z);
      continue;
    }
    const b = el('button', 'item' + (it[4] ? ' ' + it[4] : ''));
    b.setAttribute('role', 'menuitem');
    b.innerHTML = it[1];
    const l = el('span', 'label'); l.textContent = it[2];
    b.append(l);
    if (it[3]) { const h = el('span', 'hint'); h.textContent = it[3]; b.append(h); }
    b.addEventListener('click', () => send({ cmd: 'menu_action', id: it[0] }));
    root.append(b);
  }

  const fit = () => send({ cmd: 'menu_size', height: Math.ceil(root.getBoundingClientRect().height) + 2 });
  window.menu = { setState(s) { if (zv) zv.textContent = s.zoom + '%'; } };
  window.addEventListener('blur', () => send({ cmd: 'menu_close' }));
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') send({ cmd: 'menu_close' }); });
  fit();
})();
