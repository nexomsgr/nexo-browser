(() => {
  'use strict';
  const send = (m) => window.ipc.postMessage(JSON.stringify(m));
  const form = document.getElementById('f');
  const q = document.getElementById('q');
  const box = document.getElementById('sites');
  form.addEventListener('submit', (e) => {
    e.preventDefault();
    const input = q.value.trim();
    if (input) send({ cmd: 'navigate', input });
  });

  // Плитки — самые посещаемые страницы из локальной истории. Данные сайтов — только textContent.
  const hostOf = (u) => { try { return new URL(u).hostname.replace(/^www\./, ''); } catch (_) { return u; } };
  window.nexoPage = {
    update(kind, data) {
      if (kind !== 'newtab' || !data || !Array.isArray(data.sites)) return;
      box.textContent = '';
      for (const s of data.sites.slice(0, 8)) {
        const host = hostOf(s.url);
        const name = (s.title && s.title.trim()) || host;
        const b = document.createElement('button');
        b.className = 'site';
        b.title = s.url;
        const tile = document.createElement('span');
        tile.className = 'tile';
        tile.textContent = (Array.from(host)[0] || '?').toUpperCase();
        const label = document.createElement('span');
        label.className = 'name';
        label.textContent = name;
        b.append(tile, label);
        b.addEventListener('click', () => send({ cmd: 'navigate', input: s.url }));
        box.append(b);
      }
    },
  };
  send({ cmd: 'page_data' });
  q.focus();
})();
