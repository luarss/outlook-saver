// Popup orchestration: ask the content script to scrape the open email, build
// an .eml or .md file, and hand it to the downloads API.

const statusEl = document.getElementById('status');

function setStatus(msg) {
  statusEl.textContent = msg;
}

/** Filesystem-safe filename, mirroring the Tauri app's scheme. */
function sanitize(s) {
  return (s || '')
    .replace(/[/\\:*?"<>|\n\r\t]/g, '-')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 120) || 'no-subject';
}

/** YYYY-MM-DD_HHMM stamp for the current local time. */
function stamp(d = new Date()) {
  const p = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}_${p(d.getHours())}${p(d.getMinutes())}`;
}

/** Encode a UTF-8 string as a base64 data URL of the given MIME type. */
function toDataUrl(mime, text) {
  const bytes = new TextEncoder().encode(text);
  let bin = '';
  for (const b of bytes) bin += String.fromCharCode(b);
  return `data:${mime};base64,${btoa(bin)}`;
}

/** Build a minimal RFC822 .eml message from the scraped fields. */
function buildEml(email) {
  const from = email.fromEmail
    ? `${email.fromName || email.fromEmail} <${email.fromEmail}>`
    : email.fromName || 'unknown';
  const headers = [
    `From: ${from}`,
    `Subject: ${email.subject}`,
    `Date: ${new Date().toUTCString()}`,
    'MIME-Version: 1.0',
    'Content-Type: text/html; charset=utf-8',
    `X-Saved-From: ${email.capturedUrl}`,
  ];
  return headers.join('\r\n') + '\r\n\r\n' + (email.html || email.text || '');
}

/** Build a Markdown rendering of the scraped fields. */
function buildMd(email) {
  return [
    `# ${email.subject}`,
    '',
    `**From:** ${email.fromName || ''} ${email.fromEmail ? `<${email.fromEmail}>` : ''}`.trim(),
    `**Saved:** ${new Date().toISOString()}`,
    `**Source:** ${email.capturedUrl}`,
    '',
    '---',
    '',
    email.text || '',
  ].join('\n');
}

async function activeTab() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  return tab;
}

async function save(format) {
  setStatus('Reading email…');
  const tab = await activeTab();
  if (!tab || !/^https:\/\/outlook\.(office|office365|live)\.com/.test(tab.url || '')) {
    setStatus('Open an email in Outlook on the web first.');
    return;
  }

  let resp;
  try {
    resp = await chrome.tabs.sendMessage(tab.id, { type: 'SCRAPE_EMAIL' });
  } catch (e) {
    setStatus('Could not reach the page — reload Outlook and try again.');
    return;
  }
  if (!resp || !resp.ok) {
    setStatus(`Scrape failed: ${resp ? resp.error : 'no response'}`);
    return;
  }

  const email = resp.email;
  const base = `${stamp()}_${sanitize(email.subject)}`;
  const file =
    format === 'eml'
      ? { name: `${base}.eml`, url: toDataUrl('message/rfc822', buildEml(email)) }
      : { name: `${base}.md`, url: toDataUrl('text/markdown', buildMd(email)) };

  chrome.downloads.download({ url: file.url, filename: file.name, saveAs: true }, (id) => {
    if (chrome.runtime.lastError) {
      setStatus(`Download error: ${chrome.runtime.lastError.message}`);
    } else {
      setStatus(`Saved ${file.name}`);
    }
  });
}

document.getElementById('saveEml').addEventListener('click', () => save('eml'));
document.getElementById('saveMd').addEventListener('click', () => save('md'));
