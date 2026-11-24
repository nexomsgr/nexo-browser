(() => {
  'use strict';

  const $ = (id) => document.getElementById(id);
  const send = (msg) => window.ipc.postMessage(JSON.stringify(msg));

  const MB = 1024 * 1024;
  const mb = (bytes) => (bytes / MB).toFixed(bytes >= 100 * MB ? 0 : 1);

  const STATE = { active: 'Активна', background: 'Фоновая', suspended: 'Приостановлена', discarded: 'Выгружена' };
  const WHY = { audio: 'звук', downloading: 'загрузка', input: 'ввод', loading: 'загрузка страницы' };

  const dur = (s) => {
    if (s < 60) return s + ' с';
    if (s < 3600) return Math.floor(s / 60) + ' мин';
    return Math.floor(s / 3600) + ' ч ' + Math.floor((s % 3600) / 60) + ' мин';
  };

  function setStat(id, value, unit) {
    const el = $(id);
    el.textContent = value;
    if (unit) {
      const u = document.createElement('span');
      u.className = 'unit';
      u.textContent = unit;
      el.append(u);
    }
  }

  function cell(tr, text, cls) {
    const td = document.createElement('td');
    if (cls) td.className = cls;
    td.textContent = text;           // все данные выводятся только как текст
    tr.append(td);
    return td;
  }

  function renderModes(r) {
    const box = $('modes');
    const frag = document.createDocumentFragment();
    for (const m of r.modes) {
      const b = document.createElement('button');
      b.className = 'card mode' + (m.id === r.mode ? ' selected' : '');
      b.dataset.mode = m.id;
      const name = document.createElement('div');
      name.className = 'name';
      name.textContent = m.label;
      const ul = document.createElement('ul');
      const items = [
        'Приостановка фоновой вкладки: через ' + dur(m.suspend_after_secs),
        'Выгрузка из памяти: через ' + dur(m.discard_after_secs),
        m.max_live_webviews ? 'Не более ' + m.max_live_webviews + ' живых вкладок' : 'Без лимита живых вкладок',
        m.trims_hidden ? 'Сжатие памяти скрытых вкладок сразу' : 'Скрытые вкладки не сжимаются до сна',
        'Проверка сна каждые ' + dur(m.tick_secs),
      ];
      if (m.extra_browser_args) items.push('Общие процессы для вкладок одного сайта (после перезапуска, экспериментально)');
      for (const t of items) {
        const li = document.createElement('li');
        li.textContent = t;
        ul.append(li);
      }
      b.append(name, ul);
      frag.append(b);
    }
    box.replaceChildren(frag);
  }

  function renderTabs(r) {
    const body = $('tabs-body');
    if (!r.tabs.length) {
      const tr = document.createElement('tr');
      const td = cell(tr, 'Нет открытых вкладок', 'empty');
      td.colSpan = 3;
      body.replaceChildren(tr);
      return;
    }
    const frag = document.createDocumentFragment();
    for (const t of r.tabs) {
      const tr = document.createElement('tr');
      cell(tr, t.title, 'title').title = t.title;
      const st = document.createElement('td');
      const badge = document.createElement('span');
      badge.className = 'badge ' + t.state;
      badge.textContent = STATE[t.state] || t.state;
      st.append(badge);
      if (t.protected) {
        const lock = document.createElement('span');
        lock.className = 'lock';
        lock.textContent = 'не усыпляется: ' + (WHY[t.protected] || t.protected);
        st.append(lock);
      }
      tr.append(st);
      cell(tr, t.state === 'active' ? '—' : dur(t.idle_secs), 'num');
      frag.append(tr);
    }
    body.replaceChildren(frag);
  }

  function renderProcs(r) {
    const body = $('procs-body');
    if (!r.processes.length) {
      const tr = document.createElement('tr');
      const td = cell(tr, r.supported ? 'Нет данных' : 'Измерение недоступно', 'empty');
      td.colSpan = 4;
      body.replaceChildren(tr);
      return;
    }
    const frag = document.createDocumentFragment();
    for (const p of r.processes) {
      const tr = document.createElement('tr');
      cell(tr, p.name);
      cell(tr, String(p.pid), 'num');
      cell(tr, mb(p.working_set) + ' МБ', 'num');
      cell(tr, mb(p.private_bytes) + ' МБ', 'num');
      frag.append(tr);
    }
    body.replaceChildren(frag);
  }

  window.nexoPerf = {
    update(r) {
      if (!r) return;
      const un = $('unsupported');
      un.classList.toggle('show', !r.supported);
      if (!r.supported) un.textContent = 'Измерение памяти недоступно: ' + (r.error || 'неизвестная причина') + '. Значения не показываются, чтобы не вводить в заблуждение.';

      if (r.supported) {
        setStat('ws', mb(r.total_working_set), 'МБ');
        setStat('priv', mb(r.total_private_bytes), 'МБ');
        setStat('cpu', r.cpu_percent === null ? '…' : r.cpu_percent.toFixed(1), r.cpu_percent === null ? '' : '%');
        setStat('procs', String(r.process_count));
      } else {
        for (const id of ['ws', 'priv', 'cpu', 'procs']) setStat(id, '—');
      }
      setStat('tabs-total', String(r.tabs_total));
      setStat('tabs-awake', String(r.tabs_awake));
      setStat('tabs-sleep', String(r.tabs_sleeping));

      $('restart').classList.toggle('show', !!r.restart_needed);
      $('sleeping').checked = !!r.sleeping_enabled;
      renderModes(r);
      renderTabs(r);
      renderProcs(r);
      $('updated').textContent = 'Обновлено ' + new Date(r.taken_at_ms).toLocaleTimeString();
    },
    notice(text) { $('notice').textContent = text; },
  };

  $('free').addEventListener('click', () => send({ cmd: 'free_memory' }));
  $('modes').addEventListener('click', (e) => {
    const b = e.target.closest('.mode');
    if (b) send({ cmd: 'set_mode', mode: b.dataset.mode });
  });
  $('sleeping').addEventListener('change', (e) => send({ cmd: 'set_sleeping', enabled: e.target.checked }));

  // Опрашиваем только пока вкладка видна: у фоновой вкладки опрос не должен тратить CPU.
  let timer = null;
  const poll = () => send({ cmd: 'perf_snapshot' });
  function sync() {
    if (document.visibilityState === 'visible') {
      if (!timer) { poll(); timer = setInterval(poll, 2000); }
    } else if (timer) {
      clearInterval(timer);
      timer = null;
    }
  }
  document.addEventListener('visibilitychange', sync);
  sync();
})();
