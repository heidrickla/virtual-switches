// Tests tools/extract-releases.js in a headless Chromium-family browser (Edge, Chrome or Chromium;
// VSWITCH_BROWSER names another). Every page a test reads is served from this file through the DevTools
// protocol's Fetch domain, so nothing reaches cisco.com or the Wayback Machine. Run with Node 22 or later:
// node --test tools/extract-releases.test.mjs (VSWITCH_EXTRACTOR names another copy of the extractor).
import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gunzipSync } from 'node:zlib';

const here = dirname(fileURLToPath(import.meta.url));
const EXTRACTOR = readFileSync(process.env.VSWITCH_EXTRACTOR || join(here, 'extract-releases.js'), 'utf8');
if (process.env.VSWITCH_BROWSER && !existsSync(process.env.VSWITCH_BROWSER)) throw new Error(`VSWITCH_BROWSER: no ${process.env.VSWITCH_BROWSER}`);
const BROWSER = [
  process.env.VSWITCH_BROWSER,
  'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
  'C:/Program Files/Google/Chrome/Application/chrome.exe',
  '/usr/bin/chromium',
  '/usr/bin/chromium-browser',
  '/usr/bin/google-chrome',
].find((p) => p && existsSync(p));
const T = { skip: !BROWSER && 'no Chromium-family browser; VSWITCH_BROWSER names one', timeout: 60_000 };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// A DevTools protocol connection: send() answers with the command's result, listeners see every event.
class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.next = 0;
    this.waiting = new Map();
    this.listeners = new Set();
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      const w = msg.id && this.waiting.get(msg.id);
      if (w) {
        this.waiting.delete(msg.id);
        if (msg.error) w.fail(new Error(`${w.method}: ${msg.error.message}`));
        else w.ok(msg.result);
      } else if (msg.method) {
        for (const l of this.listeners) l(msg);
      }
    };
  }
  send(method, params = {}, sessionId) {
    const id = ++this.next;
    return new Promise((ok, fail) => {
      this.waiting.set(id, { ok, fail, method });
      this.ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  }
}

// one browser for the file, its profile a temporary folder
let browser;
before(async () => {
  if (!BROWSER) return;
  const profile = mkdtempSync(join(tmpdir(), 'vswitch-extract-'));
  const proc = spawn(BROWSER, ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`, '--no-first-run', '--no-default-browser-check', '--disable-extensions', '--disable-sync', '--disable-background-networking', '--disable-component-update', '--disable-breakpad', 'about:blank'], { stdio: 'ignore' });
  const portFile = join(profile, 'DevToolsActivePort');
  let lines = [];
  for (let i = 0; i < 400 && lines.length < 2; i++) {
    await sleep(50);
    if (existsSync(portFile)) lines = readFileSync(portFile, 'utf8').split('\n').filter(Boolean);
  }
  if (lines.length < 2) {
    proc.kill();
    throw new Error(`${BROWSER} wrote no DevToolsActivePort in 20 s`);
  }
  const ws = new WebSocket(`ws://127.0.0.1:${lines[0]}${lines[1]}`);
  await new Promise((ok, fail) => {
    ws.onopen = ok;
    ws.onerror = () => fail(new Error(`${BROWSER}: the DevTools socket did not open`));
  });
  browser = { proc, profile, cdp: new Cdp(ws) };
});
after(async () => {
  if (!browser) return;
  const exited = new Promise((ok) => (browser.proc.exitCode !== null ? ok() : browser.proc.once('exit', ok)));
  await Promise.race([browser.cdp.send('Browser.close').catch(() => {}), sleep(5000)]);
  await Promise.race([exited, sleep(10000)]);
  if (browser.proc.exitCode === null) {
    if (process.platform === 'win32') spawnSync('taskkill', ['/T', '/F', '/PID', String(browser.proc.pid)]);
    else browser.proc.kill('SIGKILL');
  }
  browser.cdp.ws.close();
  // Edge's helper processes hold the profile for a few seconds after the browser exits (measured
  // 2026-10-07: past 5 s twice in 16 runs); a profile left behind is named, not a failed test
  try {
    rmSync(browser.profile, { recursive: true, force: true, maxRetries: 60, retryDelay: 250 });
  } catch (e) {
    console.error(`${browser.profile} was not removed: ${e.message}`);
  }
});

// A page on `origin` in a browser context of its own (so its IndexedDB starts empty), every request it
// makes answered from `pages`: by URL, the page's HTML (200), { status, body, location }, or a list
// answered in turn, its last again after that. Any other URL answers 404.
const open = async (origin, pages) => {
  const { cdp } = browser;
  const start = `${origin}/vswitch-test.html`;
  const all = { ...pages, [start]: html('') };
  const hits = {};
  const { browserContextId } = await cdp.send('Target.createBrowserContext');
  const { targetId } = await cdp.send('Target.createTarget', { url: 'about:blank', browserContextId });
  const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
  const listener = (msg) => {
    if (msg.sessionId !== sessionId || msg.method !== 'Fetch.requestPaused') return;
    const url = msg.params.request.url;
    const n = hits[url] || 0;
    hits[url] = n + 1;
    const p = all[url];
    const a = Array.isArray(p) ? p[Math.min(n, p.length - 1)] : p;
    const r = a === undefined ? { status: 404 } : typeof a === 'string' ? { status: 200, body: a } : a;
    const headers = [{ name: 'Content-Type', value: 'text/html; charset=utf-8' }];
    if (r.location) headers.push({ name: 'Location', value: r.location });
    cdp.send('Fetch.fulfillRequest', { requestId: msg.params.requestId, responseCode: r.status, responseHeaders: headers, body: Buffer.from(r.body || '').toString('base64') }, sessionId).catch(() => {});
  };
  cdp.listeners.add(listener);
  await cdp.send('Fetch.enable', { patterns: [{ urlPattern: '*' }] }, sessionId);
  const evaluate = async (expression) => {
    const r = await cdp.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, sessionId);
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  };
  await cdp.send('Page.navigate', { url: start }, sessionId);
  for (let i = 0; ; i++) {
    const at = await evaluate('location.href + " " + document.readyState').catch(() => '');
    if (at === `${start} complete`) break;
    if (i > 400) throw new Error(`${start} did not load`);
    await sleep(25);
  }
  const close = async () => {
    cdp.listeners.delete(listener);
    await cdp.send('Target.closeTarget', { targetId });
    await cdp.send('Target.disposeBrowserContext', { browserContextId });
  };
  return { hits, evaluate, close };
};

// Runs `body` with a page from open(), closing the page after.
const withPage = async (origin, pages, body) => {
  const page = await open(origin, pages);
  try {
    await body(page);
  } finally {
    await page.close();
  }
};

// Pastes the extractor as its header says, with every pause cut to a millisecond, and waits for the run
// to end: every job done or failed, or the run stopped, and nothing running.
const extract = async (page, jobs) => {
  await page.evaluate(`window.VSWITCH_JOBS = ${JSON.stringify(jobs)};
    window.VSWITCH_PAUSE_MS = 1;
    { const wait = window.setTimeout; window.setTimeout = (f, ms, ...a) => wait(f, Math.min(ms || 0, 1), ...a); }`);
  assert.equal(await page.evaluate(EXTRACTOR), 'started');
  for (let i = 0; ; i++) {
    const s = await page.evaluate('window.__vs');
    if (!s.running.length && (s.stopped || s.done.length + (s.failed || []).length === jobs.length)) return s;
    if (i > 800) throw new Error(`the run did not end: ${JSON.stringify(s)}`);
    await sleep(25);
  }
};

const OPEN_DB = `new Promise((ok, fail) => {
  const r = indexedDB.open('vswitch-extract', 1);
  r.onupgradeneeded = () => r.result.createObjectStore('files');
  r.onsuccess = () => ok(r.result);
  r.onerror = () => fail(r.error);
})`;

// The files the store holds, by name: when each was kept (null for a bare Blob) and its JSON.
const stored = (page) => page.evaluate(`(async () => {
  const d = await ${OPEN_DB};
  const rows = await new Promise((ok, fail) => {
    const out = [];
    const c = d.transaction('files').objectStore('files').openCursor();
    c.onsuccess = () => {
      if (!c.result) return ok(out);
      out.push([c.result.key, c.result.value]);
      c.result.continue();
    };
    c.onerror = () => fail(c.error);
  });
  d.close();
  const out = {};
  for (const [name, v] of rows) {
    const gz = v instanceof Blob ? v : v.gz;
    const json = JSON.parse(await new Response(gz.stream().pipeThrough(new DecompressionStream('gzip'))).text());
    out[name] = { kept: v instanceof Blob ? null : v.kept, json };
  }
  return out;
})()`);

// Files an earlier run kept, { name: kept }: kept is a time, or null for a file kept as a bare Blob
// before the store recorded times.
const seed = (page, files) => page.evaluate(`(async () => {
  const d = await ${OPEN_DB};
  for (const [name, kept] of Object.entries(${JSON.stringify(files)})) {
    const gz = await new Response(new Blob([JSON.stringify({ platform: 'an earlier run' })]).stream().pipeThrough(new CompressionStream('gzip'))).blob();
    await new Promise((ok, fail) => {
      const t = d.transaction('files', 'readwrite');
      t.objectStore('files').put(kept === null ? gz : { gz, kept }, name);
      t.oncomplete = ok;
      t.onerror = () => fail(t.error);
    });
  }
  d.close();
})()`);

// vswitchFlush(options) with the download caught before it starts: the listing it returns, how many
// downloads it began, and the tar's bytes.
const flush = async (page, options = '') => {
  const r = await page.evaluate(`(async () => {
    const made = [];
    const objectURL = URL.createObjectURL;
    const click = HTMLAnchorElement.prototype.click;
    URL.createObjectURL = (b) => (made.push(b), objectURL(b));
    HTMLAnchorElement.prototype.click = function () {};
    try {
      const listing = await window.vswitchFlush(${options});
      const tar = made.length ? new Uint8Array(await made[0].arrayBuffer()) : new Uint8Array(0);
      let bin = '';
      for (let i = 0; i < tar.length; i += 8192) bin += String.fromCharCode(...tar.subarray(i, i + 8192));
      return { listing, downloads: made.length, tar: btoa(bin) };
    } finally {
      URL.createObjectURL = objectURL;
      HTMLAnchorElement.prototype.click = click;
    }
  })()`);
  return { ...r, entries: untar(Buffer.from(r.tar, 'base64')) };
};

// A ustar archive's entries: name, size, mtime, whether the header checksum holds, and the bytes.
const untar = (buf) => {
  const out = [];
  for (let off = 0; off + 512 <= buf.length; ) {
    const h = buf.subarray(off, off + 512);
    if (h.every((b) => b === 0)) break;
    const field = (at, n) => h.subarray(at, at + n).toString('latin1').replace(/\0[\s\S]*$/, '');
    const size = parseInt(field(124, 12), 8);
    const sum = [...h].reduce((s, b, i) => s + (i >= 148 && i < 156 ? 32 : b), 0);
    out.push({ name: field(0, 100), size, mtime: parseInt(field(136, 12), 8), sumOk: parseInt(field(148, 8), 8) === sum, data: buf.subarray(off + 512, off + 512 + size) });
    off += 512 + Math.ceil(size / 512) * 512;
  }
  return out;
};

const CISCO = 'https://www.cisco.com';
const ARCHIVE = 'https://web.archive.org';
const html = (body) => `<!doctype html><html><head><title>page</title></head><body>${body}</body></html>`;
const links = (...hrefs) => html(hrefs.map((h) => `<a href="${h}">${h}</a>`).join('\n'));
// a command as the DITA books (IOS XE) mark it up, and as the FrameMaker books (classic IOS) do
const dita = (name) => `<article class="nested1" id="${name}"><h2 class="topictitle2">${name}</h2><section class="section"><p>${name} sets it.</p></section><section class="section refsyn"><p class="synblk"><b>${name}</b> <var>value</var></p></section></article>`;
const frame = (name, syntaxClass = 'pCE_CmdEnv') => `<h2 class="pCRC_CmdRefCommand"><a name="${name}"></a>${name}</h2><p class="pB1_Body1">${name} sets it.</p><p class="${syntaxClass}">${name} <em class="cEmphasis">value</em></p>`;
// a configuration guide procedure step and its example
const step = (syntax) => `<table><tr><td><p class="cmd">${syntax}</p><div class="stepxmp"><pre>Device(config)# ${syntax}</pre></div></td></tr></table>`;
const CR = `${CISCO}/c/en/us/td/docs/switches/lan/c9300/cr/b_cr.html`;
const crPage = (c) => `${CISCO}/c/en/us/td/docs/switches/lan/c9300/cr/b_cr/${c}.html`;
const CG = `${CISCO}/c/en/us/td/docs/switches/lan/c9300/cg/b_cg.html`;
const cgPage = (c) => `${CISCO}/c/en/us/td/docs/switches/lan/c9300/cg/b_cg/${c}.html`;
const SUPPORT = `${CISCO}/c/en/us/support/switches/index.html`;

test('a FrameMaker chapter counts its command headings when none of its commands parse', T, () =>
  withPage(CISCO, {
    [CR]: links('b_cr/m_dita.html', 'b_cr/m_frame.html', 'b_cr/m_odd.html'),
    [crPage('m_dita')]: html(dita('vlan')),
    [crPage('m_frame')]: html(frame('switchport')),
    // a syntax paragraph class the parser does not know
    [crPage('m_odd')]: html(frame('spanning-tree', 'pCX_CmdSyntax')),
  }, async (page) => {
    const s = await extract(page, [{ platform: 'c9300', release: '17.15', cr: CR }]);
    assert.deepEqual([s.stopped, s.failed || []], [null, []]);
    const cr = (await stored(page))['cisco-c9300-17.15-cr.json.gz'].json;
    assert.deepEqual(cr.chapters.map((c) => [c.chapter, c.headings, c.commands.length]), [['m_dita', 1, 1], ['m_frame', 1, 1], ['m_odd', 1, 0]]);
  }));

test('a page answered from another address is named, and a chapter moved to another page is still read', T, () =>
  withPage(CISCO, {
    [CR]: links('b_cr/m_vlan.html', 'b_cr/m_gone.html'),
    [crPage('m_vlan')]: html(dita('vlan')),
    [crPage('m_gone')]: { status: 301, location: SUPPORT },
    [SUPPORT]: html('<h1>Support</h1>'),
    [CG]: links('b_cg/m_ok.html', 'b_cg/m_moved.html'),
    [cgPage('m_ok')]: html(step('vlan 10')),
    [cgPage('m_moved')]: { status: 302, location: cgPage('m_new') },
    [cgPage('m_new')]: html(step('name users')),
  }, async (page) => {
    const s = await extract(page, [{ platform: 'c9300', release: '17.15', cr: CR, cg: [CG] }]);
    const files = await stored(page);
    const cr = files['cisco-c9300-17.15-cr.json.gz'].json;
    assert.deepEqual(cr.chapters.map((c) => [c.chapter, c.status, c.redirected]), [['m_vlan', 200, undefined], ['m_gone', 200, SUPPORT]]);
    const cg = files['cisco-c9300-17.15-cg.json.gz'].json;
    assert.deepEqual(cg.books[0].chapters.map((c) => [c.url, c.redirected, c.steps.length]), [[cgPage('m_ok'), undefined, 1], [cgPage('m_moved'), cgPage('m_new'), 1]]);
    assert.deepEqual([s.done[0].notOk, s.done[0].problems], [2, [`${crPage('m_gone')}: answered from ${SUPPORT}`, `${cgPage('m_moved')}: answered from ${cgPage('m_new')}`]]);
  }));

test('a job that fails leaves the run going, and refused requests stop it', T, () => {
  const book = (b) => `${CISCO}/c/en/us/td/docs/switches/connectedgrid/cr/${b}.html`;
  const at = (url) => `${ARCHIVE}/web/2017id_/${url}`;
  return withPage(ARCHIVE, {
    // contents that list no chapters
    [at(book('b_a'))]: html('<p>Retired</p>'),
    [at(book('b_b'))]: links('b_b/m_vlan.html'),
    [at(book('b_b').replace('.html', '/m_vlan.html'))]: html(dita('vlan')),
    [at(book('b_c'))]: { status: 429 },
    [at(book('b_d'))]: links('b_d/m_vlan.html'),
  }, async (page) => {
    // an archived run has one worker, so the jobs run in this order
    const s = await extract(page, [
      { platform: 'ie3010', release: '12.2(53)EZ', archive: '2017', cr: book('b_a') },
      { platform: 'ie3010', release: '15.0(2)SE', archive: '2017', cr: book('b_b') },
      { platform: 'ie3000', release: '12.2(50)SE', archive: '2017', cr: book('b_c') },
      { platform: 'ie3000', release: '12.2(52)SE', archive: '2017', cr: book('b_d') },
    ]);
    assert.deepEqual(s.done.map((d) => d.job), ['ie3010 15.0(2)SE']);
    assert.deepEqual((s.failed || []).map((f) => f.job), ['ie3010 12.2(53)EZ', 'ie3000 12.2(50)SE']);
    assert.match(s.failed[0].error, /b_a\.html: its table of contents lists no chapters$/);
    assert.match(s.stopped, /^ie3000 12\.2\(50\)SE: Error: web\.archive\.org refused 3 requests in a row, last 429 /);
    assert.equal(page.hits[at(book('b_c'))], 3);
    assert.equal(page.hits[at(book('b_d'))], undefined);
  });
});

test('a page answered with a server error is asked again', T, () =>
  withPage(CISCO, {
    [CR]: links('b_cr/m_busy.html'),
    [crPage('m_busy')]: [{ status: 503 }, { status: 503 }, html(dita('vlan'))],
    [CG]: links('b_cg/m_busy.html'),
    [cgPage('m_busy')]: [{ status: 502 }, html(step('vlan 10'))],
  }, async (page) => {
    const s = await extract(page, [{ platform: 'c9300', release: '17.15', cr: CR, cg: [CG] }]);
    const files = await stored(page);
    const c = files['cisco-c9300-17.15-cr.json.gz'].json.chapters[0];
    const g = files['cisco-c9300-17.15-cg.json.gz'].json.books[0].chapters[0];
    assert.deepEqual([c.status, c.commands.length, g.status, g.steps.length, s.done[0].notOk], [200, 1, 200, 1, 0]);
    assert.deepEqual([page.hits[crPage('m_busy')], page.hits[cgPage('m_busy')]], [3, 2]);
  }));

test('an archived page records the capture it came from, and only an answer from a capture has one', T, () => {
  const BOOK = `${CISCO}/c/en/us/td/docs/switches/connectedgrid/ie3010/cg/b_ie3010.html`;
  const ch = (c) => `${CISCO}/c/en/us/td/docs/switches/connectedgrid/ie3010/cg/b_ie3010/${c}.html`;
  const at = (ts, url) => `${ARCHIVE}/web/${ts}id_/${url}`;
  return withPage(ARCHIVE, {
    [at('2017', BOOK)]: { status: 302, location: at('20170512093011', BOOK) },
    [at('20170512093011', BOOK)]: links('b_ie3010/m_one.html', 'b_ie3010/m_none.html', 'b_ie3010/m_retired.html'),
    [at('2017', ch('m_one'))]: { status: 302, location: at('20170512093012', ch('m_one')) },
    [at('20170512093012', ch('m_one'))]: html(step('vlan 10')),
    // no capture: the Wayback Machine answers at the address asked
    [at('2017', ch('m_none'))]: { status: 404 },
    // the capture is cisco.com's redirect to a support page
    [at('2017', ch('m_retired'))]: { status: 302, location: at('20170101000000', SUPPORT) },
    [at('20170101000000', SUPPORT)]: html('<h1>Support</h1>'),
  }, async (page) => {
    const s = await extract(page, [{ platform: 'ie3010', release: '12.2(53)EZ', archive: '2017', cg: [BOOK] }]);
    const book = (await stored(page))['cisco-ie3010-12.2(53)EZ-cg.json.gz'].json.books[0];
    assert.deepEqual([book.tocStatus, book.redirected], [200, undefined]);
    assert.deepEqual(book.chapters.map((c) => [c.url, c.status, c.capture, c.redirected]), [
      [ch('m_one'), 200, '20170512093012', undefined],
      [ch('m_none'), 404, undefined, undefined],
      [ch('m_retired'), 200, '20170101000000', SUPPORT],
    ]);
    assert.deepEqual(s.done[0].problems, [`${ch('m_none')}: status 404`, `${ch('m_retired')}: answered from ${SUPPORT}`]);
  });
});

test('a guide book whose contents list no chapters is named', T, () => {
  const EMPTY = `${CISCO}/c/en/us/td/docs/switches/lan/c9300/cg/b_empty.html`;
  return withPage(CISCO, {
    [CG]: links('b_cg/m_ok.html'),
    [cgPage('m_ok')]: html(step('vlan 10')),
    [EMPTY]: { status: 503 },
  }, async (page) => {
    const s = await extract(page, [{ platform: 'c9300', release: '17.15', cg: [CG, EMPTY] }]);
    assert.equal(page.hits[EMPTY], 3);
    assert.deepEqual([s.done[0].guideChapters, s.done[0].notOk, s.done[0].problems], [1, 1, [`${EMPTY}: its contents list no chapters (status 503)`]]);
  });
});

test('a command reference is read at its https address without doubled slashes', T, () =>
  withPage(CISCO, {
    [CR]: links('b_cr//m_vlan.html', 'http://www.cisco.com/c/en/us/td/docs/switches/lan/c9300/cr/b_cr/m_trunk.html', 'b_cr/m_vlan.html#vlan'),
    [crPage('m_vlan')]: html(dita('vlan')),
    [crPage('m_trunk')]: html(dita('switchport trunk')),
  }, async (page) => {
    const s = await extract(page, [{ platform: 'c9300', release: '17.15', cr: 'http://www.cisco.com/c/en/us/td/docs/switches/lan//c9300/cr/b_cr.html' }]);
    assert.deepEqual([s.stopped, s.failed || []], [null, []]);
    const cr = (await stored(page))['cisco-c9300-17.15-cr.json.gz'].json;
    assert.equal(cr.source, `${CISCO}/c/en/us/td/docs/switches/lan/c9300/cr/b_cr/`);
    assert.deepEqual(cr.chapters.map((c) => [c.chapter, c.commands.length]), [['m_vlan', 1], ['m_trunk', 1]]);
    const asked = Object.keys(page.hits).filter((u) => !u.endsWith('/vswitch-test.html') && !u.endsWith('/favicon.ico'));
    assert.deepEqual(asked.sort(), [CR, crPage('m_trunk'), crPage('m_vlan')].sort());
  }));

test('a job is named as grammars::BOOKS names its platform, and a platform it does not name needs a name', T, () => {
  const book = (p) => `${CISCO}/c/en/us/td/docs/switches/${p}/cg/b_cg.html`;
  const pages = {};
  for (const p of ['ie3000', 'ie3300', 'c3850a', 'c3850b']) {
    pages[book(p)] = links('b_cg/m_ok.html');
    pages[book(p).replace('.html', '/m_ok.html')] = html(step('vlan 10'));
  }
  return withPage(CISCO, pages, async (page) => {
    const s = await extract(page, [
      { platform: 'ie3000', release: '15.2(2)E', cg: [book('ie3000')] },
      { platform: 'ie3300', release: '17.6', cg: [book('ie3300')] },
      { platform: 'c3850', release: '16.12', cg: [book('c3850a')] },
      { platform: 'c3850', release: '16.9', name: 'Catalyst 3850', cg: [book('c3850b')] },
    ]);
    const files = await stored(page);
    assert.deepEqual(Object.entries(files).map(([n, f]) => [n, f.json.platform]).sort(), [
      ['cisco-c3850-16.9-cg.json.gz', 'Catalyst 3850'],
      ['cisco-ie3000-15.2(2)E-cg.json.gz', 'Cisco IE 3000'],
      ['cisco-ie3300-17.6-cg.json.gz', 'Catalyst IE3300'],
    ]);
    assert.deepEqual((s.failed || []).map((f) => f.job), ['c3850 16.12']);
    assert.match(s.failed[0].error, /c3850 is not in PLATFORMS/);
    assert.equal(page.hits[book('c3850a')], undefined);
  });
});

test('the flush holds the files this run kept, each dated when it was kept', T, () => {
  const failing = `${CISCO}/c/en/us/td/docs/switches/lan/c9200/cr/b_cr.html`;
  const earlier = Date.UTC(2026, 9, 1, 12, 0, 0);
  return withPage(CISCO, {
    [CR]: links('b_cr/m_vlan.html'),
    [crPage('m_vlan')]: html(dita('vlan')),
    [failing]: html(''),
  }, async (page) => {
    await seed(page, { 'cisco-c9200-17.15-cr.json.gz': null, 'cisco-c9300-17.12-cr.json.gz': earlier });
    const s = await extract(page, [{ platform: 'c9300', release: '17.15', cr: CR }, { platform: 'c9200', release: '17.15', cr: failing }]);
    assert.deepEqual([s.done.length, (s.failed || []).length], [1, 1]);
    const kept = (await stored(page))['cisco-c9300-17.15-cr.json.gz'].kept;
    // the run's own file: the c9200 job failed, so the earlier run's file of its name stays out
    const own = await flush(page);
    assert.equal(own.downloads, 1);
    assert.deepEqual(own.entries.map((e) => [e.name, e.mtime, e.sumOk]), [['cisco-c9300-17.15-cr.json.gz', Math.floor(kept / 1000), true]]);
    assert.equal(JSON.parse(gunzipSync(own.entries[0].data)).platform, 'Catalyst 9300');
    assert.deepEqual(own.listing, [`cisco-c9300-17.15-cr.json.gz ${own.entries[0].size} ${new Date(kept).toISOString()}`]);
    const every = await flush(page, '{ every: true }');
    assert.deepEqual(every.entries.map((e) => [e.name, e.mtime, e.sumOk]).sort(), [
      ['cisco-c9200-17.15-cr.json.gz', 0, true],
      ['cisco-c9300-17.12-cr.json.gz', earlier / 1000, true],
      ['cisco-c9300-17.15-cr.json.gz', Math.floor(kept / 1000), true],
    ]);
    const bare = every.entries.find((e) => e.name === 'cisco-c9200-17.15-cr.json.gz');
    assert.ok(every.listing.includes(`cisco-c9200-17.15-cr.json.gz ${bare.size} kept before times were recorded`));
    assert.equal(JSON.parse(gunzipSync(bare.data)).platform, 'an earlier run');
  });
});

test('a flush with nothing kept by this run downloads nothing', T, () =>
  withPage(CISCO, { [CR]: html('') }, async (page) => {
    await seed(page, { 'cisco-c9300-17.15-cr.json.gz': null });
    const s = await extract(page, [{ platform: 'c9300', release: '17.15', cr: CR }]);
    assert.equal((s.failed || []).length, 1);
    const r = await flush(page);
    assert.deepEqual([r.listing, r.downloads], [[], 0]);
  }));
