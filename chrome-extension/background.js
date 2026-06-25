// Service worker. Currently minimal — the popup does the orchestration.
//
// Reserved for the future Graph-token capture path: a webRequest/declarative
// listener could grab the bearer token OWA uses and fetch clean message JSON
// from https://graph.microsoft.com/v1.0/me/messages instead of scraping.

chrome.runtime.onInstalled.addListener(() => {
  console.log('Outlook Saver installed.');
});
