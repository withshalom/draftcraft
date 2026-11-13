// DraftCraft's service worker: it lets the browser install the web app and open it offline.
// It only answers this site's own GET requests and keeps a copy of what the app loads; it does
// no work while you draw and never touches the canvas or the GPU.
const CACHE = 'draftcraft-web';
// Trunk names each build's script and wasm with a content hash, so a file with a hash never changes.
const HASHED = /-[0-9a-f]{16}(_bg)?\.[a-z]+$/;

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()));

self.addEventListener('fetch', (event) => {
  const req = event.request;
  const url = new URL(req.url);
  if (req.method !== 'GET' || url.origin !== self.location.origin) return;
  if (req.mode === 'navigate') event.respondWith(page(event, req));
  else if (HASHED.test(url.pathname)) event.respondWith(savedFirst(req));
  else event.respondWith(networkFirst(req));
});

// The page: from the network, so a new build shows at once; the saved copy when offline. Files
// of older builds that the new page no longer names are dropped.
async function page(event, req) {
  const cache = await caches.open(CACHE);
  try {
    const res = await fetch(req);
    if (res.ok) {
      await cache.put('./', res.clone());
      event.waitUntil(res.clone().text().then((html) => prune(cache, html)));
    }
    return res;
  } catch (err) {
    return (await cache.match('./')) || Response.error();
  }
}

// A file with a hash: the saved copy if there is one, else the network.
async function savedFirst(req) {
  const cache = await caches.open(CACHE);
  const hit = await cache.match(req);
  if (hit) return hit;
  const res = await fetch(req);
  if (res.ok && res.type === 'basic') await cache.put(req, res.clone());
  return res;
}

// Any other file (icons, the manifest): the network, the saved copy when offline.
async function networkFirst(req) {
  const cache = await caches.open(CACHE);
  try {
    const res = await fetch(req);
    if (res.ok && res.type === 'basic') await cache.put(req, res.clone());
    return res;
  } catch (err) {
    return (await cache.match(req)) || Response.error();
  }
}

async function prune(cache, html) {
  for (const req of await cache.keys()) {
    const name = new URL(req.url).pathname.split('/').pop();
    if (HASHED.test(name) && !html.includes(name)) await cache.delete(req);
  }
}
