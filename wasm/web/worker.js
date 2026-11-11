// Web Worker that runs bone-infill compiled to WebAssembly.
//
// Messages in:  { id, method, url, body? }   (body: ArrayBuffer)
// Messages out: { id, status, contentType, body, name }
//               { progress: { loaded, total, done? } }  while the module downloads
//
// It is the browser-side counterpart of the native HTTP server: both hand the
// request to the same Rust function.
"use strict";

const text = new TextDecoder();
const utf8 = new TextEncoder();
let wasm = null;
let broken = null;

// The module is read chunk by chunk so that the page can show how much has
// arrived (it is the one large download), then compiled from memory. That
// also works on servers that do not label .wasm files as application/wasm.
const ready = (async () => {
  const imports = { env: { now: () => performance.now() / 1000 } };
  const response = await fetch("bone_infill_wasm.wasm");
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  // With a compressed transfer this is the compressed size, smaller than
  // what is read; the page then shows megabytes instead of a percentage.
  const total = Number(response.headers.get("Content-Length")) || 0;
  let bytes;
  if (response.body && response.body.getReader) {
    const reader = response.body.getReader();
    const chunks = [];
    let loaded = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      chunks.push(value);
      loaded += value.length;
      postMessage({ progress: { loaded, total } });
    }
    bytes = new Uint8Array(loaded);
    let at = 0;
    for (const chunk of chunks) {
      bytes.set(chunk, at);
      at += chunk.length;
    }
  } else {
    bytes = new Uint8Array(await response.arrayBuffer());
  }
  postMessage({ progress: { loaded: bytes.length, total: bytes.length, done: true } });
  const { instance } = await WebAssembly.instantiate(bytes, imports);
  wasm = instance.exports;
})();

function pass(bytes) {
  const pointer = wasm.alloc(bytes.length);
  new Uint8Array(wasm.memory.buffer, pointer, bytes.length).set(bytes);
  return pointer;
}

function fail(id, message) {
  const body = utf8.encode(JSON.stringify({ error: message })).buffer;
  postMessage({ id, status: 500, contentType: "application/json", body, name: "" }, [body]);
}

onmessage = async (event) => {
  const { id, method, url, body } = event.data;
  try {
    await ready;
  } catch (e) {
    fail(id, "could not load the WebAssembly module: " + e.message);
    return;
  }
  if (broken) {
    fail(id, broken);
    return;
  }
  try {
    const m = utf8.encode(method), u = utf8.encode(url), b = new Uint8Array(body || 0);
    // Reserve all three before taking views: growing memory detaches them.
    const pm = pass(m), pu = pass(u), pb = pass(b);
    wasm.request(pm, m.length, pu, u.length, pb, b.length);
    const memory = new Uint8Array(wasm.memory.buffer);
    const slice = (pointer, length) => memory.slice(pointer, pointer + length);
    const out = slice(wasm.reply_body(), wasm.reply_body_len());
    postMessage(
      {
        id,
        status: wasm.reply_status(),
        contentType: text.decode(slice(wasm.reply_type(), wasm.reply_type_len())),
        name: text.decode(slice(wasm.reply_name(), wasm.reply_name_len())),
        body: out.buffer,
      },
      [out.buffer],
    );
  } catch (e) {
    // A trap (out of memory, or a bug) leaves the module in an unknown
    // state: refuse further work rather than return nonsense.
    broken = "the computation failed (" + e.message + "). The part may be too large for the browser: " +
      "try a larger voxel or use the native app. Reload the page to start again.";
    fail(id, broken);
  }
};
