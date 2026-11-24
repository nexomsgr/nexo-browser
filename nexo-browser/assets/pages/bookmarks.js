(() => {
  const { send, h, open, on, ready } = window.nexoLib;
  const out = document.getElementById('out');

  function bookmarkRow(b, folders) {
    const row = h('div', 'row');
    const link = h('div', 'grow link');
    link.append(h('div', 'name', b.title || b.url), h('div', 'dim', b.url));
    link.addEventListener('click', () => open(b.url));
    const sel = h('select');
    sel.title = 'Переместить в папку';
    const root = h('option', '', 'Без папки'); root.value = '';
    sel.append(root);
    for (const f of folders) { const o = h('option', '', f.name); o.value = String(f.id); sel.append(o); }
    sel.value = b.folder == null ? '' : String(b.folder);
    sel.addEventListener('change', () => send({ cmd: 'bookmark_move', id: b.id, folder: sel.value === '' ? null : Number(sel.value) }));
    const del = h('button', 'mini', 'Удалить');
    del.addEventListener('click', () => send({ cmd: 'bookmark_remove', id: b.id }));
    row.append(link, sel, del);
    return row;
  }

  function section(title, items, folders, folder) {
    const wrap = h('div');
    const head = h('h2', '', title);
    wrap.append(head);
    const card = h('div', 'card list');
    if (folder) {
      const row = h('div', 'row');
      row.append(h('div', 'grow dim', items.length ? '' : 'Папка пуста'));
      const del = h('button', 'mini', 'Удалить папку');
      del.title = 'Закладки из папки переходят в корень';
      del.addEventListener('click', () => send({ cmd: 'folder_remove', id: folder.id }));
      row.append(del);
      card.append(row);
    }
    for (const b of items) card.append(bookmarkRow(b, folders));
    if (!folder && !items.length) card.append(h('div', 'empty', 'Закладок пока нет.'));
    wrap.append(card);
    return wrap;
  }

  on('bookmarks', (d) => {
    const frag = document.createDocumentFragment();
    frag.append(section('Все закладки', d.bookmarks.filter((b) => b.folder == null), d.folders, null));
    for (const f of d.folders) frag.append(section(f.name, d.bookmarks.filter((b) => b.folder === f.id), d.folders, f));
    out.replaceChildren(frag);
  });

  document.getElementById('folder-form').addEventListener('submit', (e) => {
    e.preventDefault();
    const i = document.getElementById('folder-name');
    if (i.value.trim()) { send({ cmd: 'folder_add', name: i.value }); i.value = ''; }
  });
  ready();
})();
