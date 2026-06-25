const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const dialog = window.__TAURI__.dialog;

const el = (id) => document.getElementById(id);

const ui = {};
let busy = false;

function logLine(message) {
  const li = document.createElement("li");
  const time = new Date().toLocaleTimeString();
  li.textContent = `[${time}] ${message}`;
  ui.log.prepend(li);
  // Keep the log from growing unbounded.
  while (ui.log.childElementCount > 200) ui.log.lastChild.remove();
}

function setStatus(text) {
  ui.statusLine.textContent = text;
}

function applyStatus(s) {
  // Account
  ui.account.textContent = s.signed_in
    ? s.email || "Signed in"
    : "Not signed in";
  ui.loginBtn.hidden = s.signed_in;
  ui.logoutBtn.hidden = !s.signed_in;

  // Settings fields
  ui.clientId.value = s.client_id || "";
  ui.tenant.value = s.tenant || "common";
  ui.saveDir.value = s.default_save_dir || "";
  ui.askEach.checked = !!s.ask_each_time;

  // Watcher
  ui.watchState.textContent = s.watching ? "Running" : "Stopped";
  ui.startBtn.hidden = s.watching;
  ui.stopBtn.hidden = !s.watching;
  ui.startBtn.disabled = !s.signed_in || s.watching;
}

async function refresh() {
  try {
    const s = await invoke("get_status");
    applyStatus(s);
  } catch (e) {
    setStatus(`Could not read status: ${e}`);
  }
}

async function withBusy(label, fn) {
  if (busy) return;
  busy = true;
  setStatus(label);
  try {
    await fn();
  } catch (e) {
    setStatus(String(e));
    logLine(`Error: ${e}`);
  } finally {
    busy = false;
    await refresh();
  }
}

async function saveSettings() {
  await invoke("save_settings", {
    clientId: ui.clientId.value,
    tenant: ui.tenant.value,
    defaultSaveDir: ui.saveDir.value || null,
    askEachTime: ui.askEach.checked,
  });
  setStatus("Settings saved.");
  logLine("Settings saved.");
}

window.addEventListener("DOMContentLoaded", () => {
  Object.assign(ui, {
    account: el("account"),
    watchState: el("watch-state"),
    statusLine: el("status-line"),
    log: el("log"),
    loginBtn: el("login-btn"),
    logoutBtn: el("logout-btn"),
    startBtn: el("start-btn"),
    stopBtn: el("stop-btn"),
    clientId: el("client-id"),
    tenant: el("tenant"),
    saveDir: el("save-dir"),
    askEach: el("ask-each"),
    pickDir: el("pick-dir"),
    saveSettingsBtn: el("save-settings-btn"),
  });

  ui.loginBtn.addEventListener("click", () =>
    withBusy("Opening browser for sign-in…", async () => {
      await saveSettings(); // ensure client id is persisted first
      await invoke("login");
      logLine("Signed in.");
      setStatus("Signed in.");
    }),
  );

  ui.logoutBtn.addEventListener("click", () =>
    withBusy("Signing out…", async () => {
      await invoke("logout");
      logLine("Signed out.");
    }),
  );

  ui.startBtn.addEventListener("click", () =>
    withBusy("Starting watcher…", async () => {
      await invoke("start_watching");
      logLine("Watcher started.");
    }),
  );

  ui.stopBtn.addEventListener("click", () =>
    withBusy("Stopping watcher…", async () => {
      await invoke("stop_watching");
      logLine("Watcher stopped.");
    }),
  );

  ui.saveSettingsBtn.addEventListener("click", () =>
    withBusy("Saving settings…", saveSettings),
  );

  ui.pickDir.addEventListener("click", async () => {
    try {
      const dir = await dialog.open({ directory: true, multiple: false });
      if (dir) ui.saveDir.value = dir;
    } catch (e) {
      setStatus(`Could not open folder picker: ${e}`);
    }
  });

  // Backend events.
  listen("watcher-status", (e) => {
    if (e.payload && e.payload.message) setStatus(e.payload.message);
    refresh();
  });
  listen("log", (e) => {
    if (e.payload && e.payload.message) logLine(e.payload.message);
  });
  listen("mail-saved", (e) => {
    if (e.payload && e.payload.name) logLine(`Saved ${e.payload.name}`);
  });

  refresh();
});
