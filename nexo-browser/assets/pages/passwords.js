(() => {
  'use strict';
  const { send, h, on, ready, when } = window.nexoLib;
  const root = document.getElementById('root');
  const noticeEl = document.getElementById('notice');
  const fileEl = document.getElementById('csv-file');

  let st = { available: true, initialized: false, unlocked: false, items: [] };
  let mode = 'list';        // list | edit | export | master | reset
  let editing = null;       // запись в форме редактирования (null — новая)
  let query = '';
  const shown = new Map();  // id → пароль, показанный в списке (стираем при любой перерисовке из состояния)
  const pending = new Map();// id → что сделать, когда придёт пароль: copy | show | edit

  function notice(ok, text) {
    noticeEl.textContent = text;
    noticeEl.className = 'notice show' + (ok ? '' : ' err');
  }
  const clearNotice = () => { noticeEl.className = 'notice'; noticeEl.textContent = ''; };

  const field = (labelText, input) => { const l = h('label', '', labelText); l.append(input); return l; };
  const pwInput = (ph) => { const i = h('input'); i.type = 'password'; i.autocomplete = 'off'; i.spellcheck = false; if (ph) i.placeholder = ph; return i; };
  const btn = (text, cls, fn) => { const b = h('button', cls || 'btn', text); b.type = 'button'; b.addEventListener('click', fn); return b; };

  async function copy(text) {
    try { await navigator.clipboard.writeText(text); return true; }
    catch (_) {
      const t = document.createElement('textarea'); t.value = text; document.body.append(t); t.select();
      let ok = false; try { ok = document.execCommand('copy'); } catch (__) { /* нет доступа к буферу */ }
      t.remove(); return ok;
    }
  }

  // ---------- Экраны ----------
  function screenCreate() {
    const card = h('div', 'card form');
    card.append(h('div', 'folder', 'Создайте мастер-пароль'));
    const a = pwInput(), b = pwInput();
    card.append(field('Мастер-пароль (минимум 8 символов)', a), field('Повторите мастер-пароль', b));
    card.append(h('div', 'hint', 'Мастер-пароль нигде не хранится. Если вы его забудете, пароли восстановить будет невозможно — останется только удалить хранилище.'));
    const go = () => {
      if (a.value !== b.value) return notice(false, 'Пароли не совпадают');
      if (a.value.length < 8) return notice(false, 'Мастер-пароль должен быть не короче 8 символов');
      clearNotice(); send({ cmd: 'vault_create', master: a.value }); a.value = b.value = '';
    };
    const act = h('div', 'actions'); act.append(btn('Создать хранилище', 'btn', go));
    card.append(act);
    b.addEventListener('keydown', (e) => { if (e.key === 'Enter') go(); });
    root.replaceChildren(card);
    a.focus();
  }

  function screenUnlock() {
    const card = h('div', 'card form');
    card.append(h('div', 'folder', 'Хранилище заблокировано'));
    const a = pwInput();
    card.append(field('Мастер-пароль', a));
    const go = () => { if (!a.value) return; clearNotice(); send({ cmd: 'vault_unlock', master: a.value }); a.value = ''; };
    const act = h('div', 'actions');
    act.append(btn('Разблокировать', 'btn', go), btn('Забыл пароль…', 'mini', () => { mode = 'reset'; render(); }));
    card.append(act);
    a.addEventListener('keydown', (e) => { if (e.key === 'Enter') go(); });
    root.replaceChildren(card);
    a.focus();
  }

  function screenReset() {
    const card = h('div', 'card form');
    card.append(h('div', 'folder', 'Удалить хранилище?'));
    card.append(h('div', 'hint', 'Мастер-пароль восстановить невозможно. Можно только безвозвратно удалить все сохранённые пароли и начать заново.'));
    const act = h('div', 'actions');
    act.append(btn('Удалить все пароли', 'btn warn', () => { mode = 'list'; send({ cmd: 'vault_reset' }); }), btn('Отмена', 'btn ghost', () => { mode = 'list'; render(); }));
    card.append(act);
    root.replaceChildren(card);
  }

  function screenEdit() {
    const it = editing || { id: null, name: '', url: '', username: '', password: '', note: '' };
    const card = h('div', 'card form');
    card.append(h('div', 'folder', it.id == null ? 'Новый пароль' : 'Изменить запись'));
    const name = h('input'), url = h('input'), user = h('input'), pw = pwInput(), note = h('textarea');
    for (const i of [name, url, user]) { i.type = 'text'; i.autocomplete = 'off'; i.spellcheck = false; }
    name.value = it.name; url.value = it.url; user.value = it.username; pw.value = it.password; note.value = it.note || '';
    const pwRow = h('div', 'pw'); pwRow.append(pw,
      btn('Показать', 'mini', () => { pw.type = pw.type === 'password' ? 'text' : 'password'; }),
      btn('Создать', 'mini', () => { pw.value = generate(); pw.type = 'text'; }));
    card.append(field('Название', name), field('Адрес сайта', url), field('Логин', user), field('Пароль', pwRow), field('Заметка', note));
    const save = () => {
      if (!pw.value) return notice(false, 'Введите пароль');
      if (!url.value.trim() && !name.value.trim()) return notice(false, 'Укажите адрес сайта или название');
      clearNotice();
      send({ cmd: 'vault_save', id: it.id, name: name.value.trim(), url: url.value.trim(), username: user.value, password: pw.value, note: note.value });
      editing = null; mode = 'list'; render();
    };
    const act = h('div', 'actions');
    act.append(btn('Сохранить', 'btn', save), btn('Отмена', 'btn ghost', () => { editing = null; mode = 'list'; render(); }));
    card.append(act);
    root.replaceChildren(card);
    (it.id == null ? name : pw).focus();
  }

  // Криптостойкий генератор: crypto.getRandomValues с отбраковкой смещения.
  function generate(len = 20) {
    const set = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%^&*-_=+';
    const out = []; const buf = new Uint32Array(1); const lim = Math.floor(0x100000000 / set.length) * set.length;
    while (out.length < len) { crypto.getRandomValues(buf); if (buf[0] < lim) out.push(set[buf[0] % set.length]); }
    return out.join('');
  }

  function screenExport() {
    const card = h('div', 'card form');
    card.append(h('div', 'folder', 'Экспорт паролей в CSV'));
    card.append(h('div', 'hint', 'Файл будет содержать ВСЕ пароли в открытом виде (формат Chrome/Brave: name, url, username, password, note) и сохранится в папку «Загрузки». Любой, кто получит файл, увидит ваши пароли — удалите его сразу после переноса. Для подтверждения введите мастер-пароль.'));
    const a = pwInput(); card.append(field('Мастер-пароль', a));
    const go = () => { if (!a.value) return; send({ cmd: 'vault_export', master: a.value }); a.value = ''; mode = 'list'; render(); };
    const act = h('div', 'actions'); act.append(btn('Экспортировать', 'btn warn', go), btn('Отмена', 'btn ghost', () => { mode = 'list'; render(); }));
    card.append(act);
    a.addEventListener('keydown', (e) => { if (e.key === 'Enter') go(); });
    root.replaceChildren(card); a.focus();
  }

  function screenMaster() {
    const card = h('div', 'card form');
    card.append(h('div', 'folder', 'Смена мастер-пароля'));
    const o = pwInput(), n = pwInput(), n2 = pwInput();
    card.append(field('Текущий мастер-пароль', o), field('Новый мастер-пароль (минимум 8 символов)', n), field('Повторите новый', n2));
    const go = () => {
      if (n.value !== n2.value) return notice(false, 'Новые пароли не совпадают');
      if (n.value.length < 8) return notice(false, 'Мастер-пароль должен быть не короче 8 символов');
      send({ cmd: 'vault_change_master', old: o.value, new: n.value }); o.value = n.value = n2.value = ''; mode = 'list'; render();
    };
    const act = h('div', 'actions'); act.append(btn('Сменить', 'btn', go), btn('Отмена', 'btn ghost', () => { mode = 'list'; render(); }));
    card.append(act); root.replaceChildren(card); o.focus();
  }

  function screenList() {
    const bar = h('div', 'toolbar');
    const q = h('input'); q.type = 'text'; q.placeholder = 'Поиск по названию, адресу, логину'; q.value = query;
    q.addEventListener('input', () => { query = q.value; renderRows(); });
    bar.append(q,
      btn('Добавить', 'btn', () => { editing = null; mode = 'edit'; render(); }),
      btn('Импорт CSV', 'btn ghost', () => fileEl.click()),
      btn('Экспорт CSV', 'btn ghost', () => { mode = 'export'; render(); }),
      btn('Мастер-пароль', 'btn ghost', () => { mode = 'master'; render(); }),
      btn('Заблокировать', 'btn ghost', () => { send({ cmd: 'vault_lock' }); }));
    const card = h('div', 'card list'); card.id = 'rows';
    root.replaceChildren(bar, card);
    renderRows();
  }

  function renderRows() {
    const card = document.getElementById('rows');
    if (!card) return;
    const s = query.trim().toLowerCase();
    const list = st.items.filter((c) => !s || (c.name + ' ' + c.url + ' ' + c.username).toLowerCase().includes(s));
    card.replaceChildren();
    if (!list.length) { card.append(h('div', 'empty', st.items.length ? 'Ничего не найдено.' : 'Паролей пока нет. Добавьте вручную или импортируйте CSV из другого браузера.')); return; }
    for (const c of list) {
      const row = h('div', 'row');
      const g = h('div', 'grow');
      g.append(h('div', 'name', c.name || c.url), h('div', 'dim', (c.username || '—') + (c.url && c.name ? ' · ' + c.url : '')));
      if (shown.has(c.id)) g.append(h('div', 'pwv', shown.get(c.id)));
      const a = h('div', 'acts');
      const act = (label, fn) => a.append(btn(label, 'mini', fn));
      act('Логин', async () => { const ok = await copy(c.username); notice(ok, ok ? 'Логин скопирован' : 'Не удалось скопировать'); });
      act('Пароль', () => { pending.set(c.id, 'copy'); send({ cmd: 'vault_reveal', id: c.id }); });
      act(shown.has(c.id) ? 'Скрыть' : 'Показать', () => {
        if (shown.has(c.id)) { shown.delete(c.id); renderRows(); } else { pending.set(c.id, 'show'); send({ cmd: 'vault_reveal', id: c.id }); }
      });
      act('Изменить', () => { pending.set(c.id, 'edit'); send({ cmd: 'vault_reveal', id: c.id }); });
      const del = btn('Удалить', 'mini danger', () => {
        if (del.dataset.sure) { send({ cmd: 'vault_delete', id: c.id }); return; }
        del.dataset.sure = '1'; del.textContent = 'Точно?'; setTimeout(() => { delete del.dataset.sure; del.textContent = 'Удалить'; }, 3000);
      });
      a.append(del);
      row.append(g, a);
      card.append(row);
    }
  }

  function render() {
    if (!st.available) { root.replaceChildren(h('div', 'card empty', 'В приватном окне менеджер паролей недоступен: там ничего не сохраняется.')); return; }
    if (mode === 'reset') return screenReset();
    if (!st.initialized) return screenCreate();
    if (!st.unlocked) return screenUnlock();
    if (mode === 'edit') return screenEdit();
    if (mode === 'export') return screenExport();
    if (mode === 'master') return screenMaster();
    return screenList();
  }

  on('passwords', (s) => {
    const wasUnlocked = st.unlocked;
    st = s;
    if (!s.unlocked) { shown.clear(); pending.clear(); editing = null; if (wasUnlocked && mode !== 'reset') mode = 'list'; }
    // Не перерисовываем форму, пока в ней идёт ввод.
    if (['edit', 'export', 'master'].includes(mode) && s.unlocked) return;
    if (mode === 'list' && s.unlocked && document.getElementById('rows')) { renderRows(); return; }
    render();
  });
  on('notice', (n) => notice(n.ok, n.text));
  on('reveal', async (r) => {
    const what = pending.get(r.id); pending.delete(r.id);
    if (what === 'copy') notice(true, (await copy(r.password)) ? 'Пароль скопирован в буфер обмена. Очистите буфер после использования.' : 'Не удалось скопировать');
    else if (what === 'show') { shown.set(r.id, r.password); renderRows(); setTimeout(() => { shown.delete(r.id); renderRows(); }, 15000); }
    else if (what === 'edit') {
      const it = st.items.find((c) => c.id === r.id);
      if (it) { editing = Object.assign({}, it, { password: r.password }); mode = 'edit'; render(); }
    }
  });

  fileEl.addEventListener('change', async (e) => {
    const f = e.target.files[0]; e.target.value = '';
    if (!f) return;
    notice(true, 'Импортирую…');
    try { await window.nexoLib.sendFile('passwords', f); } catch (err) { notice(false, err.message); }
  });

  document.addEventListener('visibilitychange', () => { if (!document.hidden) send({ cmd: 'vault_status' }); });
  ready();
})();
