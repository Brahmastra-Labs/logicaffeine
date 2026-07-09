// Make the browser `WebSocket` global available to web-sys under node, so the
// relay + interpreter networking wasm-bindgen tests run without a browser.
const WS = require('ws');
globalThis.WebSocket = WS.WebSocket || WS;
