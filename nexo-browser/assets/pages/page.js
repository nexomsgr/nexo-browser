'use strict';
// Общие помощники внутренних страниц. Все данные от сайтов выводятся только как текст (textContent).
(() => {
  const send = (msg) => window.ipc.postMessage(JSON.stringify(msg));
  const h = (tag, cls, text) => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  };
  const size = (n) => {
    if (n == null) return '';
    const u = ['Б', 'КБ', 'МБ', 'ГБ', 'ТБ'];
    let i = 0, v = n;
    while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
    return (i === 0 ? v : v.toFixed(v >= 100 ? 0 : 1)) + ' ' + u[i];
  };
  const when = (ms) => new Date(ms).toLocaleString();
  const open = (url) => send({ cmd: 'navigate', input: url });
  const handlers = {};
  window.nexoPage = { update(kind, data) { if (handlers[kind]) handlers[kind](data); } };
  // Навигация между внутренними страницами (ссылки — через команду navigate, не через href).
  const NAV = [['Закладки', 'bookmarks'], ['История', 'history'], ['Загрузки', 'downloads'], ['Пароли', 'passwords'], ['Настройки', 'settings'], ['Производительность', 'performance']];
  const here = location.pathname.replace(/^\//, '');
  const page = document.querySelector('.page');
  if (page) {
    const nav = h('div', 'toolbar');
    for (const [label, id] of NAV) {
      const b = h('button', id === here ? 'btn' : 'btn ghost', label);
      if (id !== here) b.addEventListener('click', () => open('nexo://' + id));
      nav.append(b);
    }
    page.prepend(nav);
  }
  // Передача файла в Rust кусками (лимит одного сообщения — 128 КБ). Режем по символам Unicode, а не пополам
  // суррогатной пары: иначе каждый кусок станет невалидным JSON.
  const CHUNK = 24000;
  const MAX_FILE = 32 * 1024 * 1024;
  async function sendFile(kind, file, extra) {
    if (!file) return false;
    if (file.size > MAX_FILE) throw new Error('Файл больше 32 МБ');
    const text = await file.text();
    send({ cmd: 'import_begin', kind });
    for (let i = 0; i < text.length;) {
      let end = Math.min(i + CHUNK, text.length);
      const last = text.charCodeAt(end - 1);
      if (end < text.length && last >= 0xd800 && last <= 0xdbff) end++;
      send({ cmd: 'import_chunk', data: text.slice(i, end) });
      i = end;
    }
    send(Object.assign({ cmd: 'import_commit', kind }, extra || {}));
    return true;
  }
  window.nexoLib = {
    send, h, size, when, open, sendFile,
    on(kind, fn) { handlers[kind] = fn; },
    // Первый запрос данных; страница сама просит их, когда готова их принять.
    ready() { send({ cmd: 'page_data' }); },
  };
})();
