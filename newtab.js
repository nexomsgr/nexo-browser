(() => {
  'use strict';
  const form = document.getElementById('f');
  const q = document.getElementById('q');
  form.addEventListener('submit', (e) => {
    e.preventDefault();
    const input = q.value.trim();
    if (input) window.ipc.postMessage(JSON.stringify({ cmd: 'navigate', input }));
  });
  q.focus();
})();
