// Extracts Catalyst command references and configuration guides from cisco.com, one JSON file per book
// set. Run in a browser tab on cisco.com (cisco.com refuses scripted HTTP clients). Set window.VSWITCH_JOBS
// to a list of { platform: 'c9300', release: '17.15', cr: <command reference URL>, cg: [<guide URLs>] }
// (either of cr and cg may be left out; `name` names a platform PLATFORMS does not, `archive` below) and
// paste this file; it starts the jobs and returns at once. window.__vs holds the progress: done (each
// with the pages not read as asked), failed, the jobs running, and a stop reason. Each finished book set
// is kept gzip-compressed in this origin's IndexedDB (`vswitch-extract`) as JSON whose top level names its
// platform, release and kind; once the run is done, `await window.vswitchFlush()` downloads the files
// this run kept as one tar, a single download for the whole run (`vswitchFlush({ every: true })`: every
// file this browser has kept), each dated when it was kept. Unpack it to
// reference/cisco-<release>/<platform>-<cr|cg>.json (not committed). A run of refused requests stops the
// extraction instead of retrying; any other failure is its job's alone and the run goes on.
//
// A book cisco.com has retired is read from the Wayback Machine: run in a tab on web.archive.org with a
// job's `archive` set to a timestamp prefix (`'2017'`); each page is fetched as archived
// (`/web/<timestamp>id_/<url>`, the bytes cisco.com served) from the capture nearest that time, and each
// chapter records the capture it came from. The index (`/cdx/search/cdx?url=<prefix>&matchType=prefix`)
// lists the pages archived under a book.
//
// Command reference, per command: name, anchor id, first description sentence, syntax lines (<var> marks
// an argument), and each section (Syntax Description, Command Default, Command Modes, Command History,
// Usage Guidelines, Examples) as text, table rows and preformatted blocks. The chapters come from the
// book's table of contents.
// Configuration guides, per chapter: each procedure step's syntax (the step's `p.cmd`) with its example
// lines (`.stepxmp pre`), and every preformatted block (configuration examples, show output).
(() => {
  const archived = (window.VSWITCH_JOBS || []).some((j) => j.archive);
  // the Wayback Machine refuses a client that reads as fast as cisco.com allows
  const PAUSE_MS = window.VSWITCH_PAUSE_MS || (archived ? 1500 : 300);
  const WORKERS = archived ? 1 : 4;
  const STOP_AFTER_REFUSALS = 3;
  // the files this run keeps are those kept from now on
  const run = Date.now();
  const state = (window.__vs = { done: [], failed: [], running: [], stopped: null, refused: 0, requests: 0 });
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  // the page's own scripts wrap window.fetch after load; keep the one in place now
  const fetchPage = window.fetch.bind(window);
  const WAYBACK = /^https:\/\/web\.archive\.org\/web\/(\d+)id_\/(.*)$/;
  // `url` and the URL answered are cisco.com's; `capture` is the archived copy's timestamp, which only an
  // answer from a capture has (one at the requested address, no capture or refused, has the job's prefix)
  const get = async (url, archive) => {
    await sleep(PAUSE_MS);
    state.requests++;
    const r = await fetchPage(archive ? `https://web.archive.org/web/${archive}id_/${url}` : url, { credentials: 'include' });
    const text = await r.text();
    if (r.status === 403 || r.status === 429) {
      state.refused++;
      if (state.refused >= STOP_AFTER_REFUSALS) throw Object.assign(new Error(`${archive ? 'web.archive.org' : 'cisco.com'} refused ${state.refused} requests in a row, last ${r.status} ${url}`), { refused: true });
    } else {
      state.refused = 0;
    }
    const w = r.url.match(WAYBACK);
    if (!w) return { status: r.status, url: r.url, text };
    return { status: r.status, url: w[2].replace(/^http:/, 'https:').replace(':80/', '/'), ...(/^\d{14}$/.test(w[1]) ? { capture: w[1] } : {}), text };
  };
  // cisco.com answers a page 503 now and then (two of 94 guide pages on 2026-10-06); a server error is
  // asked again before the page is kept as it came
  const getPage = async (url, archive) => {
    let r = await get(url, archive);
    for (let attempt = 0; attempt < 3 && r.status >= 500; attempt++) {
      await sleep(3000);
      r = await get(url, archive);
    }
    return r;
  };
  // links resolve against the page's own address, not this tab's (relative links, archived pages)
  const parse = (html, url) => {
    const doc = new DOMParser().parseFromString(html, 'text/html');
    if (url) {
      const b = doc.createElement('base');
      b.href = url;
      doc.head.prepend(b);
    }
    return doc;
  };
  // cisco.com links some books as http:// or with a doubled slash; chapters are matched on the https form
  const norm = (u) => {
    const x = new URL(u, location.href);
    x.protocol = 'https:';
    x.pathname = x.pathname.replace(/\/{2,}/g, '/');
    x.hash = '';
    return x.href;
  };
  // the address a page was answered from when it is not the one asked for: a retired chapter redirected
  // to a support page, or an archived redirect
  const moved = (r, url) => (norm(r.url) !== norm(url) ? { redirected: r.url } : {});
  const ser = (el) => {
    let s = '';
    for (const n of el.childNodes) {
      if (n.nodeType === 3) s += n.textContent;
      else if (n.nodeType === 1) {
        const cls = n.className || '';
        // FrameMaker books (2960, 12.2(44)SE to 15.2(4)E) mark an argument em.cEmphasis or em.cCi_CmdItalic
        if (n.tagName === 'VAR' || /varname/.test(cls) || (n.tagName === 'EM' && /cEmphasis|cCi_CmdItalic|CommandItalic|(^|\s)i(\s|$)/.test(cls))) s += '<' + n.textContent.trim() + '>';
        else if (n.tagName === 'KBD' && /sep/.test(cls)) s += ' ';
        else s += ser(n);
      }
    }
    return s;
  };
  const clean = (t) => t.replace(/\s+/g, ' ').trim();
  // each book's platform name as grammars::BOOKS has it: a grammar finds its book by this name
  const PLATFORMS = {
    c9300: 'Catalyst 9300',
    c9200: 'Catalyst 9200',
    c9400: 'Catalyst 9400',
    c2960x: 'Catalyst 2960x',
    c2960: 'Catalyst 2960',
    ie3300: 'Catalyst IE3300',
    ie3000: 'Cisco IE 3000',
    ie3010: 'Cisco IE 3010',
  };
  const platformName = (job) => {
    const name = job.name || PLATFORMS[job.platform];
    if (!name) throw new Error(`${job.platform} is not in PLATFORMS: set the job's \`name\` to the platform name grammars::BOOKS will give it`);
    return name;
  };
  // IE3x00 guides write a step as prose. The 17.1 guide puts its syntax in the info section after it
  // (keywords, arguments em.ph.i); the area guides in bold inside the prose, arguments unmarked: a
  // hyphenated word no example line holds is taken as an argument.
  const stepSyntax = (cmd, cell, examples) => {
    const own = clean(ser(cmd));
    const kwds = [...cmd.querySelectorAll('.kwd')];
    const first = kwds.length ? clean(kwds[0].textContent).split(' ')[0].toLowerCase() : '';
    if (kwds.length && own.toLowerCase().startsWith(first)) return own;
    if (kwds.length) {
      const text = clean(kwds.map((k) => k.textContent).join(' '));
      const seen = examples.join(' ').toLowerCase();
      return text.replace(/[A-Za-z][\w-]*/g, (w) => (/[-_]/.test(w) && !seen.includes(w.toLowerCase()) ? `<${w}>` : w));
    }
    const info = [...cell.querySelectorAll('.itemgroup.info p')].find((q) => q.querySelector('.kwd') && clean(q.textContent).toLowerCase().startsWith(clean(q.querySelector('.kwd').textContent).toLowerCase()));
    if (!info) return own;
    // a line that runs on into its description after a dash ends at the dash
    const s = clean(ser(info)).split('—')[0].trim();
    return (s.match(/</g) || []).length > (s.match(/>/g) || []).length ? s + '>' : s;
  };
  // a FrameMaker example: one section (a div in the IE 3000 and IE 3010 15.x books) per line, a block
  // while the lines are siblings (12.2(44)SE puts an empty anchor between them)
  const EX = '.pEx1_Example1, .pEx2_Example2, .pEx3_Example3';
  const exampleBlocks = (lines) => {
    const blocks = [];
    for (const e of lines) {
      const t = e.textContent.replace(/\u00a0/g, ' ').trim();
      let prev = e.previousElementSibling;
      while (prev && prev.tagName === 'A' && !prev.textContent.trim()) prev = prev.previousElementSibling;
      if (blocks.length && prev && prev.matches(EX)) blocks[blocks.length - 1] += '\n' + t;
      else blocks.push(t);
    }
    return blocks;
  };
  // FrameMaker command references: a command runs from its heading to the next one in document order, its
  // Command Modes and later sections in sibling sections; a heading with no syntax line is no command, a
  // section heading with no text continues the section before it (12.2(44)SE)
  const HEADS = 'h2.pCRC_CmdRefCommand, h2.p_H_Head1';
  const frameCommands = (doc) => {
    const all = [...doc.body.querySelectorAll('*')];
    const heads = all.map((e, i) => (e.matches(HEADS) ? i : -1)).filter((i) => i >= 0);
    const isTitle = (e) => /^H[23]$/.test(e.tagName) && /^pCR/.test(e.className) && clean(e.textContent) !== '';
    const cmds = [];
    heads.forEach((start, k) => {
      const seg = all.slice(start + 1, k + 1 < heads.length ? heads[k + 1] : all.length);
      const first = seg.findIndex(isTitle);
      const before = first < 0 ? seg : seg.slice(0, first);
      // the syntax paragraph's class is pCE_CmdEnv, pCENB_CmdEnv_NoBold, or pCE_CmdE (IE 15.x books)
      const syn = before.filter((e) => e.tagName === 'P' && /(^|\s)pCE/.test(e.className));
      if (!syn.length) return;
      const h = all[start];
      const anchor = h.querySelector('a[name]');
      const intro = before.find((e) => e.matches('p.pB1_Body1'));
      const cmd = { name: clean(h.textContent), id: anchor ? anchor.getAttribute('name') : '', intro: intro ? clean(intro.textContent) : '', syntax: [...new Set(syn.map((p) => clean(ser(p))).filter(Boolean))], sections: {} };
      const titles = seg.map((e, i) => (isTitle(e) ? i : -1)).filter((i) => i >= 0);
      titles.forEach((t, j) => {
        const body = seg.slice(t + 1, j + 1 < titles.length ? titles[j + 1] : seg.length);
        const sec = { text: [], rows: [], pre: exampleBlocks(body.filter((e) => e.matches(EX))) };
        for (const e of body) {
          if (e.closest(EX)) continue;
          if (e.tagName === 'TR') sec.rows.push([...e.querySelectorAll('td, th')].map((td) => clean(ser(td))));
          else if (e.tagName === 'PRE') sec.pre.push(e.textContent);
          else if (e.tagName === 'P' && !e.closest('table')) {
            const x = clean(e.textContent);
            if (x) sec.text.push(x);
          }
        }
        cmd.sections[clean(seg[t].textContent)] = sec;
      });
      cmds.push(cmd);
    });
    return cmds;
  };
  // Results wait in IndexedDB and leave in one download: each download costs the hosting browser window
  // resources it keeps until it restarts, and a run per release would make dozens.
  const db = () => new Promise((ok, fail) => {
    const r = indexedDB.open('vswitch-extract', 1);
    r.onupgradeneeded = () => r.result.createObjectStore('files');
    r.onsuccess = () => ok(r.result);
    r.onerror = () => fail(r.error);
  });
  const keep = async (name, blob) => {
    const gz = await new Response(blob.stream().pipeThrough(new CompressionStream('gzip'))).blob();
    const d = await db();
    await new Promise((ok, fail) => {
      const t = d.transaction('files', 'readwrite');
      t.objectStore('files').put({ gz, kept: Date.now() }, name + '.gz');
      t.oncomplete = ok;
      t.onerror = () => fail(t.error);
    });
  };
  const download = (blob, name) => {
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = name;
    document.body.appendChild(a);
    a.click();
    a.remove();
  };
  // the store keeps every run's files: a job that fails leaves an earlier run's file of its name, which
  // only `every` takes, dated when that run kept it
  window.vswitchFlush = async ({ every = false } = {}) => {
    const d = await db();
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
    // a file kept before the store recorded times is a bare Blob, dated 0
    const files = rows
      .map(([name, v]) => (v instanceof Blob ? { name, gz: v, kept: 0 } : { name, gz: v.gz, kept: v.kept }))
      .filter((f) => every || f.kept >= run);
    // no download for nothing: each one opens a Save As dialog in the app's browser pane
    if (!files.length) return [];
    // a ustar archive: a 512-byte header per file, its bytes padded to 512, two empty blocks at the end
    const enc = new TextEncoder();
    const parts = [];
    for (const { name, gz, kept } of files) {
      const h = new Uint8Array(512);
      const put = (s, off) => h.set(enc.encode(s), off);
      put(name.slice(0, 99), 0);
      put('0000644\0', 100);
      put('0000000\0', 108);
      put('0000000\0', 116);
      put(gz.size.toString(8).padStart(11, '0') + '\0', 124);
      put(Math.floor(kept / 1000).toString(8).padStart(11, '0') + '\0', 136);
      put('        ', 148);
      put('0', 156);
      put('ustar\0' + '00', 257);
      put(h.reduce((a, b) => a + b, 0).toString(8).padStart(6, '0') + '\0 ', 148);
      parts.push(h, gz, new Uint8Array((512 - (gz.size % 512)) % 512));
    }
    parts.push(new Uint8Array(1024));
    download(new Blob(parts, { type: 'application/x-tar' }), 'vswitch-extract.tar');
    return files.map((f) => `${f.name} ${f.gz.size} ${f.kept ? new Date(f.kept).toISOString() : 'kept before times were recorded'}`);
  };
  const save = async (obj, name) => {
    await keep(name, new Blob([JSON.stringify(obj)], { type: 'application/json' }));
  };

  const commandReference = async (job) => {
    const platform = platformName(job);
    const cr = norm(job.cr);
    const base = cr.replace(/\.html$/, '/');
    let chapters = [];
    let landed = cr;
    for (let attempt = 0; attempt < 3 && !chapters.length; attempt++) {
      if (attempt) await sleep(5000);
      const t = await get(cr, job.archive);
      landed = t.url;
      chapters = [...new Set([...parse(t.text, cr).querySelectorAll('a')]
        .map((a) => norm(a.href))
        .filter((h) => h.startsWith(base) && h.endsWith('.html') && !/_CLT_chapter/.test(h))
        .map((h) => h.slice(base.length, -'.html'.length)))];
    }
    if (!chapters.length) throw new Error(`${cr}: its table of contents lists no chapters${norm(landed) !== cr ? `; answered from ${landed}` : ''}`);
    const out = { kind: 'cr', release: job.release, platform, source: base, chapters: [] };
    for (const c of chapters) {
      const url = base + c + '.html';
      const r = await getPage(url, job.archive);
      const doc = parse(r.text, url);
      const frame = !!doc.querySelector('h2.pCRC_CmdRefCommand');
      const cmds = frame ? frameCommands(doc) : [];
      // every command heading against the commands kept: a gap is a page the parser does not read, so
      // the headings counted are the page's kind, whatever the parser made of it
      const headings = doc.querySelectorAll(frame ? HEADS : 'h2.topictitle2, h3.topictitle3').length;
      // newer books put each command in article.nested1 with p.synblk syntax lines; older ones (2960-X
      // 15.2(2)E, 15.2(4)E) in section.nested1, the syntax as plain paragraphs under an `__refsyn_` anchor
      for (const art of cmds.length ? [] : doc.querySelectorAll('article.nested1, section.nested1')) {
        const h = art.querySelector('h2.topictitle2, h3.topictitle3');
        if (!h) continue;
        const anchor = art.querySelector('a[name]');
        const cmd = { name: clean(h.textContent), id: art.id || (anchor ? anchor.getAttribute('name') : ''), intro: '', syntax: [], sections: {} };
        const introP = art.querySelector('section.section:not(.refsyn) p');
        if (introP) cmd.intro = clean(introP.textContent);
        let syn = [...art.querySelectorAll('section.refsyn p.synblk, p.synblk')];
        const refsyn = art.querySelector('a[name*="__refsyn"]');
        if (!syn.length && refsyn) syn = [...refsyn.parentElement.querySelectorAll('p')];
        cmd.syntax = [...new Set(syn.map((p) => clean(ser(p))).filter(Boolean))];
        const titles = [...art.querySelectorAll('.sectiontitle')];
        for (let i = 0; i < titles.length; i++) {
          const key = clean(titles[i].textContent);
          const sec = { text: [], rows: [], pre: [] };
          let n = titles[i].nextElementSibling;
          while (n && !n.classList.contains('sectiontitle') && !(n.querySelector && n.querySelector('.sectiontitle'))) {
            for (const tr of n.querySelectorAll ? n.querySelectorAll('tr') : []) sec.rows.push([...tr.querySelectorAll('td, th')].map((td) => clean(ser(td))));
            for (const pre of n.querySelectorAll ? n.querySelectorAll('pre') : []) sec.pre.push(pre.textContent);
            if (n.tagName === 'PRE') sec.pre.push(n.textContent);
            if (!n.querySelector || !n.querySelector('tr')) {
              const t = clean(n.textContent);
              if (t) sec.text.push(t);
            }
            n = n.nextElementSibling;
          }
          cmd.sections[key] = sec;
        }
        cmds.push(cmd);
      }
      out.chapters.push({ chapter: c, url, status: r.status, ...moved(r, url), bytes: r.text.length, headings, ...(r.capture ? { capture: r.capture } : {}), commands: cmds });
    }
    return out;
  };

  const guides = async (job) => {
    const out = { kind: 'cg', release: job.release, platform: platformName(job), books: [] };
    for (const book of job.cg.map(norm)) {
      const bookDir = book.replace(/\.html$/, '/');
      // a table of contents that lists no chapters is fetched again: it came back without its links
      let chapters = [];
      let tocStatus = 0;
      let landed = book;
      for (let attempt = 0; attempt < 3 && !chapters.length; attempt++) {
        if (attempt) await sleep(5000);
        const t = await get(book, job.archive);
        tocStatus = t.status;
        landed = t.url;
        chapters = [...new Set([...parse(t.text, book).querySelectorAll('a')].map((a) => norm(a.href)).filter((h) => h.startsWith(bookDir) && h.endsWith('.html')))];
      }
      // a retired book answers with a redirect to a support page
      const b = { book, tocStatus, chapters: [], ...(norm(landed) !== book ? { redirected: landed } : {}) };
      for (const url of chapters) {
        const r = await getPage(url, job.archive);
        const doc = parse(r.text, url);
        const steps = [];
        for (const cmd of doc.querySelectorAll('td p.cmd, td p.ph.cmd')) {
          const cell = cmd.closest('td');
          const examples = [...cell.querySelectorAll('.stepxmp pre')].flatMap((p) => p.textContent.split('\n')).map((l) => l.replace(/\s+/g, ' ').trim()).filter(Boolean);
          steps.push({ syntax: stepSyntax(cmd, cell, examples), examples });
        }
        // FrameMaker guides: the step number in its own cell, the command in the next
        if (!steps.length) {
          for (const n of doc.querySelectorAll('td p.pCSF_CellStepFirst, td p.pCSN_CellStepnext')) {
            const p = n.closest('td').nextElementSibling?.querySelector('p');
            if (p) steps.push({ syntax: clean(ser(p)), examples: [] });
          }
        }
        const pres = [...doc.querySelectorAll('pre')].filter((p) => !p.closest('.stepxmp') && !p.closest(EX)).map((p) => p.textContent).concat(exampleBlocks([...doc.querySelectorAll(EX)]));
        b.chapters.push({ url, status: r.status, ...moved(r, url), ...(r.capture ? { capture: r.capture } : {}), title: doc.title, steps, pres });
      }
      out.books.push(b);
    }
    return out;
  };

  // what a job read that is not a page as asked: a status other than 200 after the retries, an answer from
  // another address, a guide book whose contents listed no chapters
  const problems = (cr, cg) => {
    const page = (c) => [...(c.status !== 200 ? [`${c.url}: status ${c.status}`] : []), ...(c.redirected ? [`${c.url}: answered from ${c.redirected}`] : [])];
    return [
      ...cr.chapters.flatMap(page),
      ...cg.books.flatMap((b) => [
        ...(b.chapters.length ? [] : [`${b.book}: its contents list no chapters (status ${b.tocStatus}${b.redirected ? `, answered from ${b.redirected}` : ''})`]),
        ...b.chapters.flatMap(page),
      ]),
    ];
  };

  // jobs run WORKERS at a time, each fetching one page after another: about the parallelism of one page
  // load in a browser
  const queue = [...(window.VSWITCH_JOBS || [])];
  state.running = [];
  const worker = async () => {
    while (queue.length && !state.stopped) {
      const job = queue.shift();
      const name = `${job.platform} ${job.release}`;
      state.running.push(name);
      const kept = [];
      try {
        // a job names the command reference, the guides or both; the other file is left as it is
        const cr = job.cr ? await commandReference(job) : { chapters: [] };
        if (job.cr) {
          await save(cr, `cisco-${job.platform}-${job.release}-cr.json`);
          kept.push('cr');
        }
        const cg = job.cg ? await guides(job) : { books: [] };
        if (job.cg) await save(cg, `cisco-${job.platform}-${job.release}-cg.json`);
        const found = problems(cr, cg);
        state.done.push({
          job: name,
          chapters: cr.chapters.length,
          commands: cr.chapters.reduce((n, c) => n + c.commands.length, 0),
          headings: cr.chapters.reduce((n, c) => n + c.headings, 0),
          guideChapters: cg.books.reduce((n, b) => n + b.chapters.length, 0),
          steps: cg.books.reduce((n, b) => n + b.chapters.reduce((m, c) => m + c.steps.length, 0), 0),
          notOk: found.length,
          ...(found.length ? { problems: found } : {}),
        });
      } catch (e) {
        // refused requests stop the run; any other failure is this job's alone
        state.failed.push({ job: name, error: String(e), ...(kept.length ? { kept } : {}) });
        if (e.refused) state.stopped = state.stopped || `${name}: ${e}`;
      }
      state.running = state.running.filter((r) => r !== name);
    }
  };
  for (let i = 0; i < WORKERS; i++) worker();
  return 'started';
})();
