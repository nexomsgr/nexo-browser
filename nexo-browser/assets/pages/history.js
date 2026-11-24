(() => {
  const { send, h, when, open, on, ready } = window.nexoLib;
  const out = document.getElementById('out');
  const q = document.getElementById('q');

  on('history', (d) => {
    if (document.activeElement !== q && q.value !== d.query) q.value = d.query;
    if (!d.entries.length) { out.replaceChildren(h('div', 'empty', d.query ? 'Ничего не найдено.' : 'История пуста.')); return; }
    const frag = document.createDocumentFragment();
    for (const e of d.entries) {
      const row = h('div', 'row');
      const link = h('div', 'grow link');
      link.append(h('div', 'name', e.title || e.url), h('div', 'dim', e.url + ' · ' + when(e.last_ms) + (e.visits > 1 ? ' · визитов: ' + e.visits : '')));
      link.addEventListener('click', () => open(e.url));
      const del = h('button', 'mini', 'Удалить');
      del.addEventListener('click', () => send({ cmd: 'history_delete', id: e.id }));
      row.append(link, del);
      frag.append(row);
    }
    out.replaceChildren(frag);
  });

  let timer = null;
  q.addEventListener('input', () => {
    clearTimeout(timer);
    timer = setTimeout(() => send({ cmd: 'history_search', query: q.value }), 200);
  });

  // Очистка необратима: нужно нажать дважды.
  const clear = document.getElementById('clear');
  let armed = null;
  clear.addEventListener('click', () => {
    if (armed) { clearTimeout(armed); armed = null; clear.textContent = 'Очистить историю'; clear.classList.remove('warn'); send({ cmd: 'history_clear' }); return; }
    clear.textContent = 'Нажмите ещё раз — удалить всё';
    clear.classList.add('warn');
    armed = setTimeout(() => { armed = null; clear.textContent = 'Очистить историю'; clear.classList.remove('warn'); }, 4000);
  });
  ready();
})();
