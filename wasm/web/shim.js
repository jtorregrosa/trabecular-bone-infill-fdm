// Runs the app page without a server: its `fetch("api/...")` calls are
// answered by a Web Worker running the WebAssembly build. Everything else on
// the page is exactly what the native server serves.
(function () {
  "use strict";
  const worker = new Worker("worker.js");
  const pending = new Map();
  let next = 1;
  worker.onmessage = (event) => {
    // Download progress of the module, for the page's loading screen.
    if (event.data.progress) {
      window.dispatchEvent(new CustomEvent("studio:progress", { detail: event.data.progress }));
      return;
    }
    const { id, status, contentType, body, name } = event.data;
    const headers = { "Content-Type": contentType };
    if (name) headers["X-File-Name"] = encodeURIComponent(name);
    pending.get(id)(new Response(body, { status, headers }));
    pending.delete(id);
  };
  worker.onerror = (event) => {
    const body = JSON.stringify({ error: "the worker stopped: " + event.message });
    for (const resolve of pending.values()) resolve(new Response(body, { status: 500, headers: { "Content-Type": "application/json" } }));
    pending.clear();
  };

  async function call(url, options) {
    let body = options && options.body;
    if (body instanceof Blob) body = await body.arrayBuffer();
    else if (typeof body === "string") body = new TextEncoder().encode(body).buffer;
    return new Promise((resolve) => {
      const id = next++;
      pending.set(id, resolve);
      worker.postMessage({ id, method: (options && options.method) || "GET", url: "/" + url, body }, body ? [body] : []);
    });
  }

  const nativeFetch = window.fetch.bind(window);
  window.fetch = (url, options) => (typeof url === "string" && url.startsWith("api/") ? call(url, options) : nativeFetch(url, options));

  // Download links point at the API too: turn them into saved files.
  document.addEventListener("click", async (event) => {
    const link = event.target.closest && event.target.closest('a[href^="api/download"]');
    if (!link) return;
    event.preventDefault();
    const response = await call(link.getAttribute("href"));
    if (!response.ok) return;
    const save = document.createElement("a");
    save.href = URL.createObjectURL(await response.blob());
    save.download = decodeURIComponent(response.headers.get("X-File-Name") || "") || link.download || "model";
    save.click();
    setTimeout(() => URL.revokeObjectURL(save.href), 30000);
  });
})();
