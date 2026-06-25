// Content script injected into Outlook on the web.
//
// Its only job: when asked, scrape the email currently open in the reading
// pane and return its fields. OWA's markup is obfuscated and changes often, so
// every selector below has fallbacks and we degrade gracefully — a missing
// field comes back empty rather than throwing.

/**
 * Try a list of selectors in order, returning the first matching element.
 * @param {string[]} selectors
 * @param {ParentNode} root
 * @returns {Element|null}
 */
function firstMatch(selectors, root = document) {
  for (const sel of selectors) {
    const el = root.querySelector(sel);
    if (el) return el;
  }
  return null;
}

/** The reading pane region, used to scope the other lookups. */
function readingPane() {
  return firstMatch([
    'div[aria-label="Reading Pane"]',
    'div[aria-label="Message body"]',
    '[role="main"]',
  ]) || document.body;
}

/** Extract the message body element (the rich HTML of the email itself). */
function bodyElement() {
  return firstMatch([
    'div[aria-label="Message body"]',
    'div[id^="UniqueMessageBody"]',
    'div.allowTextSelection',
    'div[role="document"]',
  ]);
}

/** Best-effort subject: reading-pane heading, else the tab title. */
function extractSubject(pane) {
  const heading = firstMatch(
    ['[role="heading"][aria-level="2"]', '[role="heading"]', 'span[data-testid="subject"]'],
    pane
  );
  const text = heading && heading.textContent.trim();
  if (text) return text;
  // Tab title is usually "Subject - email@addr - Outlook".
  const fromTitle = document.title.split(' - ')[0].trim();
  return fromTitle || '(no subject)';
}

/**
 * Best-effort sender. OWA renders the sender persona in a few shapes; we look
 * for an explicit email address anywhere in the persona header.
 */
function extractFrom(pane) {
  const persona = firstMatch(
    [
      'span[automation-id="SenderPersona"]',
      'div[data-testid="SenderPersona"]',
      'button[aria-label^="From"]',
    ],
    pane
  );
  const scope = persona || pane;
  // Pull an email out of any title attribute or visible text.
  const titled = scope.querySelector('[title*="@"]');
  if (titled) {
    const m = (titled.getAttribute('title') || '').match(/[^\s<>]+@[^\s<>]+/);
    if (m) return { name: titled.textContent.trim() || m[0], email: m[0] };
  }
  const m = scope.textContent.match(/[^\s<>]+@[^\s<>]+\.[^\s<>]+/);
  return { name: m ? m[0] : '', email: m ? m[0] : '' };
}

/** Scrape the currently-open email into a plain object. */
function scrapeEmail() {
  const pane = readingPane();
  const body = bodyElement();
  const from = extractFrom(pane);
  return {
    subject: extractSubject(pane),
    fromName: from.name,
    fromEmail: from.email,
    html: body ? body.innerHTML : '',
    text: body ? body.innerText : pane.innerText,
    // No reliable Date header in the DOM; the background fills in "now".
    capturedUrl: location.href,
  };
}

chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg && msg.type === 'SCRAPE_EMAIL') {
    try {
      sendResponse({ ok: true, email: scrapeEmail() });
    } catch (e) {
      sendResponse({ ok: false, error: String(e) });
    }
  }
  // Synchronous response; no need to return true.
});
