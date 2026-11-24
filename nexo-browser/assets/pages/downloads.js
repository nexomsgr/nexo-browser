(() => {
  const { send, h, size, on, ready } = window.nexoLib;
  const out = document.getElementById('out');
  const STATE = { in_progress: 'Загружается', completed: 'Готово', cancelled: 'Отменено', failed: 'Ошибка' };

  on('downloads', (d) => {
    if (!d.items.length) { out.replaceChildren(h('div', 'empty', 'Загрузок пока нет.')); return; }
    const frag = document.createDocumentFragment();
    for (const i of d.items) {
      const row = h('div', 'row');
      const main = h('div', 'grow');
      main.append(h('div', 'name', i.name));
      const info = i.state === 'in_progress'
        ? size(i.received) + (i.total ? ' из ' + size(i.total) : '')
        : (i.state === 'completed' ? size(i.received || i.total) : '');
      main.append(h('div', 'dim', info));
      if (i.state === 'in_progress') {
        const bar = h('div', 'bar' + (i.progress == null ? ' unknown' : ''));
        const fill = h('i');
        if (i.progress != null) fill.style.width = Math.round(i.progress * 100) + '%';
        bar.append(fill);
        main.append(bar);
      }
      row.append(main, h('span', 'badge ' + i.state, STATE[i.state] || i.state));
      if (i.state === 'in_progress') {
        const b = h('button', 'mini', 'Отмена');
        b.addEventListener('click', () => send({ cmd: 'download_cancel', id: i.id }));
        row.append(b);
      } else if (i.state === 'completed') {
        const b = h('button', 'mini', 'Показать в папке');
        b.addEventListener('click', () => send({ cmd: 'download_show', id: i.id }));
        row.append(b);
      }
      frag.append(row);
    }
    out.replaceChildren(frag);
  });
  document.getElementById('clear').addEventListener('click', () => send({ cmd: 'downloads_clear_finished' }));
  ready();
})();
