# Outlook Saver — Chrome extension

Saves the email you're currently viewing in **Outlook on the web** as an `.eml`
or `.md` file. Unlike the Tauri/IMAP app in this repo, it needs no Azure app
registration and no IMAP access — it rides your already-authenticated browser
session.

## Load it (unpacked)

1. Go to `chrome://extensions`.
2. Enable **Developer mode** (top right).
3. **Load unpacked** → select this `chrome-extension/` folder.
4. Open an email in <https://outlook.office.com>, click the extension icon,
   then **Save .eml** or **Save .md**.

## How it works

- `content_script.js` scrapes the open email from the reading pane (subject,
  sender, body HTML/text).
- `popup.js` builds the file and triggers a download.
- `background.js` is a placeholder for a future Microsoft Graph token-capture
  path (cleaner data than scraping).

## Known limitations

- **Manual, one-at-a-time.** It saves the email you're looking at on a click —
  there's no background auto-save (MV3 can't hold an IMAP-style connection).
  Use the Tauri app for "save every new email automatically."
- **Scraping is brittle.** OWA's markup changes often. If a save comes back
  empty or wrong, the selectors in `content_script.js` likely need updating.
- **No real Date header** — `.eml` uses the save time, not the original
  received time. The Graph path would fix this.
- Icons are omitted; Chrome shows a default placeholder.
