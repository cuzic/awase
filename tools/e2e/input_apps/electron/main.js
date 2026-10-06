// Electron(Chromium)の <textarea>/<input> 1 つ。引数: textarea|input、本文の書き出し先。
const { app, BrowserWindow } = require('electron');
const fs = require('fs');
const kind = process.argv[2] === 'input' ? 'input' : 'textarea';
const out = process.argv[3];
app.whenReady().then(() => {
  fs.writeFileSync(out, '');
  const w = new BrowserWindow({ width: 760, height: 300, x: 100, y: 100, title: 'electron-form-input',
    webPreferences: { nodeIntegration: true, contextIsolation: false } });
  w.setTitle('electron-form-input');
  w.on('page-title-updated', (e) => e.preventDefault());
  const html = `<!doctype html><meta charset=utf-8><title>electron-form-input</title>
    <${kind} id=t ${kind === 'textarea' ? 'rows=8 cols=80' : 'size=80'} autofocus></${kind}>
    <script>const fs=require('fs');const t=document.getElementById('t');
    t.addEventListener('input',()=>fs.writeFileSync(${JSON.stringify(out)},t.value));t.focus();</script>`;
  w.loadURL('data:text/html;charset=utf-8,' + encodeURIComponent(html));
});
app.on('window-all-closed', () => app.quit());
