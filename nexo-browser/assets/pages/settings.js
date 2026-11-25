(() => {
  const { send, h, on, ready, open } = window.nexoLib;
  const $ = (id) => document.getElementById(id);
  const zoom = $('zoom');
  for (const p of [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300]) {
    const o = h('option', '', p + ' %'); o.value = String(p); zoom.append(o);
  }
  const update = (patch) => send(Object.assign({ cmd: 'update_settings' }, patch));

  on('settings', (s) => {
    $('theme').value = s.theme;
    $('engine').value = s.search_engine;
    $('restore').checked = !!s.restore_session;
    if (document.activeElement !== $('home')) $('home').value = s.homepage;
    // Если сохранён масштаб не из списка — добавляем его, чтобы значение не терялось.
    if (![...zoom.options].some((o) => o.value === String(s.default_zoom_pct))) {
      const o = h('option', '', s.default_zoom_pct + ' %'); o.value = String(s.default_zoom_pct); zoom.append(o);
    }
    zoom.value = String(s.default_zoom_pct);
  });

  $('theme').addEventListener('change', (e) => update({ theme: e.target.value }));
  $('engine').addEventListener('change', (e) => update({ search_engine: e.target.value }));
  zoom.addEventListener('change', (e) => update({ default_zoom_pct: Number(e.target.value) }));
  $('restore').addEventListener('change', (e) => update({ restore_session: e.target.checked }));
  $('home').addEventListener('change', (e) => update({ homepage: e.target.value }));
  $('perf').addEventListener('click', () => open('nexo://performance'));
  $('to-pass').addEventListener('click', () => open('nexo://passwords'));
  $('bk-export').addEventListener('click', () => { $('bk-notice').textContent = 'Экспортирую…'; send({ cmd: 'backup_export' }); });
  $('bk-import').addEventListener('click', () => $('bk-file').click());
  $('bk-file').addEventListener('change', async (e) => {
    const f = e.target.files[0];
    e.target.value = '';
    if (!f) return;
    $('bk-notice').textContent = 'Импортирую…';
    try { await window.nexoLib.sendFile('backup', f, { apply_settings: $('bk-settings').checked }); }
    catch (err) { $('bk-notice').textContent = err.message; }
  });
  on('notice', (n) => { $('bk-notice').textContent = n.text; });
  ready();
})();
