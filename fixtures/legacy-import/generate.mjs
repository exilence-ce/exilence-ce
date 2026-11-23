// Generates the legacy import (F21) fixtures by driving the REAL released legacy apps over the
// Chrome DevTools Protocol, derived from the research lab script
// (migration repo, branch research/indexeddb-import, research/indexeddb-import/lab/cdp.mjs).
// Every value is written through `objectStore('keyvaluepairs').put(jsonString, key)`, the call
// localForage's IndexedDB driver makes, or by the app itself (mobx-persist re-writes hydrated
// stores on launch). All data is synthetic and deterministic; the GGG token is a fake.
//
// Usage: node generate.mjs <ce-root> <next-root> <work-dir>
//   <ce-root>/<next-root>: `--appimage-extract`ed squashfs-root of the CE v1.3.0 and
//   Exilence Next 1.2.2 AppImages. <work-dir>: scratch dir under /tmp holding the isolated HOMEs.
// Output: ./both/{exilence-ce-app,exilence-next-app}, ./old-migration/exilence-next-app and
// ./expected/<set>/<app>/<key>.json (the exact JSON string each key holds, read in-page).
//
// Each app runs with `env -i` and HOME = XDG_CONFIG_HOME's parent under <work-dir>, so the user's
// real legacy data (~/.config/exilence-ce-app, …) is never read or modified.
import { spawn } from 'node:child_process';
import { cpSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const [ceRoot, nextRoot, work] = process.argv.slice(2).map((p) => resolve(p));
const out = dirname(fileURLToPath(import.meta.url));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------- app session over CDP

async function session(appRoot, appName, home, port, steps) {
  mkdirSync(join(home, '.config'), { recursive: true });
  const child = spawn(join(appRoot, appName), ['--no-sandbox', `--remote-debugging-port=${port}`], {
    env: { PATH: '/usr/bin:/bin', DISPLAY: process.env.DISPLAY ?? ':0', HOME: home, XDG_CONFIG_HOME: join(home, '.config') },
    stdio: 'ignore',
  });
  const exited = new Promise((r) => child.on('exit', r));
  let page;
  for (let i = 0; i < 120 && !page; i++) {
    await sleep(500);
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      page = list.find((t) => t.type === 'page' && t.url.startsWith('file://') && t.url.includes('index.html'));
    } catch {}
  }
  if (!page) throw new Error('no page target');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r) => ws.addEventListener('open', r, { once: true }));
  let id = 0;
  const pending = new Map();
  ws.addEventListener('message', (e) => {
    const m = JSON.parse(e.data);
    pending.get(m.id)?.(m);
  });
  const evaluate = (expression) =>
    new Promise((r) => {
      const i = ++id;
      pending.set(i, r);
      ws.send(JSON.stringify({ id: i, method: 'Runtime.evaluate', params: { expression, awaitPromise: true, returnByValue: true } }));
    }).then((m) => {
      if (m.result?.exceptionDetails) throw new Error(JSON.stringify(m.result.exceptionDetails));
      return m.result.result.value;
    });
  const prelude = `
    const req = (r) => new Promise((ok, ko) => { r.onsuccess = () => ok(r.result); r.onerror = () => ko(r.error); });
    const db = await req(indexedDB.open('exilence-next-db'));
    const store = (mode) => db.transaction('keyvaluepairs', mode).objectStore('keyvaluepairs');`;
  const app = {
    // One readwrite transaction: put every [key, json] pair, then delete `del`.
    put: (pairs, del = []) =>
      evaluate(`(async () => { ${prelude}
        const s = store('readwrite');
        for (const [k, v] of ${JSON.stringify(pairs)}) s.put(v, k);
        for (const k of ${JSON.stringify(del)}) s.delete(k);
        await new Promise((ok, ko) => { s.transaction.oncomplete = ok; s.transaction.onerror = () => ko(s.transaction.error); });
      })()`),
    // Every key with its stored value, as the page sees it.
    dump: async () =>
      JSON.parse(
        await evaluate(`(async () => { ${prelude}
          const s = store('readonly');
          const keys = await req(s.getAllKeys());
          const out = {};
          for (const k of keys) out[k] = await req(s.get(k));
          return JSON.stringify(out); })()`)
      ),
    // Inline ~60 KB values until LevelDB flushes its memtable to a `.ldb` table (the log
    // rotates), then deleted again: leaves a table plus tombstones, and keeps the fixture small.
    padAndDelete: async () => {
      const dir = join(home, '.config', appName, 'IndexedDB', 'file__0.indexeddb.leveldb');
      const pads = [];
      while (!readdirSync(dir).some((f) => f.endsWith('.ldb'))) {
        if (pads.length > 400) throw new Error('no memtable flush');
        const batch = Array.from({ length: 5 }, (_, i) => ['pad' + (pads.length + i), 'x'.repeat(60000) + (pads.length + i)]);
        await app.put(batch);
        pads.push(...batch);
        await sleep(200);
      }
      await app.put([], pads.map(([k]) => k));
    },
    ipc: (channel, value) =>
      evaluate(`require('electron').ipcRenderer.send(${JSON.stringify(channel)}, ${JSON.stringify(value)})`),
  };
  await sleep(4000); // let the app hydrate and re-write its stores
  const result = await steps(app);
  ws.close();
  child.kill('SIGTERM');
  await exited;
  await sleep(500);
  return result;
}

// ---------------------------------------------------------------- deterministic synthetic data

const uuid = (tag, n) => {
  const hex = [...Buffer.from(`${tag}:${n}`)].reduce((h, b) => (h * 31 + b) >>> 0, 7).toString(16).padStart(8, '0');
  return `${hex}-${n.toString(16).padStart(4, '0')}-4000-8000-${tag.padEnd(12, '0').slice(0, 12).replace(/[^0-9a-f]/g, 'a')}`;
};

const tabs = (names) =>
  names.map(([id, name, type], index) => ({ id, index, name, type, metadata: { colour: ['ff0000', '00ff00', '0000ff'][index % 3] } }));

// `withCoffinBeast`: CE ≥ 1.2.10 items carry `coffin` and `beast`; Exilence Next / CE 1.2.2 items lack them.
function item(tag, n, tab, withCoffinBeast) {
  const it = {
    uuid: uuid(tag + 'i', n), name: '', itemId: uuid(tag + 'g', n).replace(/-/g, ''), typeLine: 'Chaos Orb', frameType: 5,
    total: 10 + n, calculated: 1, max: 1, elder: false, shaper: false, blighted: false,
    mean: 1, median: 1, min: 1, mode: 1, ilvl: 0, stackSize: 10 + n, totalStacksize: 5000, links: 0, quality: 0, level: 0,
    corrupted: false, icon: 'https://web.poecdn.com/gen/image/x/y/CurrencyRerollRare.png', sockets: 0, variant: '', tier: 0,
    inventoryId: 'Stash' + n, tab: tab ? [{ id: tab.id, name: tab.name, index: tab.index, color: tab.metadata.colour }] : [],
  };
  if (withCoffinBeast) Object.assign(it, { coffin: false, beast: false });
  return it;
}

// Newest first, like legacy (`unshift`); only the newest 10 keep `pricedItems`.
function snapshots(tag, count, startMs, tabList, withCoffinBeast) {
  return Array.from({ length: count }, (_, i) => {
    const age = count - 1 - i;
    const parts = [...tabList.map((t) => ({ id: t.id, tab: t })), { id: 'Character', tab: null }];
    return {
      uuid: uuid(tag + 's', i),
      created: new Date(startMs + age * 3_600_000).toISOString(),
      stashTabSnapshots: parts.map((p, t) => ({
        uuid: uuid(tag + 't', i * 10 + t),
        stashTabId: p.id,
        value: Math.round((1000 + age * 7.25 + t * 13.5) * 100) / 100,
        pricedItems: i < 10 ? Array.from({ length: 6 }, (_, n) => item(tag + i + 't' + t, n, p.tab, withCoffinBeast)) : [],
      })),
    };
  });
}

const profile = (uuid_, name, league, extra) => ({
  uuid: uuid_, name, activeLeagueId: league, activePriceLeagueId: league, activeCharacterName: 'None',
  activeStashTabIds: [], snapshots: [], active: false, includeEquipment: false, includeInventory: false, ...extra,
});

const fakeToken = { accessToken: 'fake-token-never-imported', expires: '2026-12-31T00:00:00.000Z', tokenType: 'bearer', scope: 'account:profile' };
const sharedProfile = uuid('shared', 1);

// Exilence Next 1.2.2 (`exilence-next-app`): account `Foo`, one Profile with 20 Snapshots.
const nextTabs = tabs([['a1b2c3d4e5', 'Currency', 'CurrencyStash'], ['f6a7b8c9d0', 'Maps', 'MapStash']]);
const nextAccount = (snapCount) => ({
  accounts: [{
    uuid: uuid('nextacc', 1), name: 'Foo', sessionId: '',
    accountLeagues: [{ uuid: uuid('nextal', 1), leagueId: 'Standard', characters: [], stashtabs: nextTabs }],
    profiles: [profile(sharedProfile, 'Standard main', 'Standard', {
      activeStashTabIds: nextTabs.map((t) => t.id), active: true, includeEquipment: true,
      snapshots: snapshots('next', snapCount, Date.UTC(2023, 8, 1), nextTabs, false),
    })],
  }],
  activeAccount: uuid('nextacc', 1),
  token: fakeToken,
});

// CE v1.3.0 (`exilence-ce-app`): `Foo` (the same Profile, re-fetched by CE 1.2.3, 2 newer
// Snapshots) and `Foo#1234` (a Profile with 18 Snapshots and a UTF-16 tab name).
const ceTabs = tabs([['0a1b2c3d4e', 'Währung ✓ 通貨', 'CurrencyStash'], ['5f6a7b8c9d', 'Dump', 'NormalStash']]);
const ceAccount = (snapCount) => ({
  accounts: [
    {
      uuid: uuid('ceacc', 1), name: 'Foo', sessionId: '',
      accountLeagues: [{ uuid: uuid('ceal', 1), leagueId: 'Standard', characters: [], stashtabs: nextTabs }],
      profiles: [profile(sharedProfile, 'Standard main (CE)', 'Standard', {
        activeStashTabIds: [nextTabs[0].id], active: true,
        snapshots: snapshots('ceshared', 2, Date.UTC(2023, 9, 1), [nextTabs[0]], true),
      })],
    },
    {
      uuid: uuid('ceacc', 2), name: 'Foo#1234', sessionId: '',
      accountLeagues: [{ uuid: uuid('ceal', 2), leagueId: 'Settlers', characters: [], stashtabs: ceTabs }],
      profiles: [
        profile(uuid('ceprof', 1), '', 'Settlers', { activeCharacterName: '', active: false }),
        profile(uuid('ceprof', 2), 'Settlers', 'Settlers', {
          activeCharacterName: 'FooChar', activeStashTabIds: ceTabs.map((t) => t.id), active: true, includeInventory: true,
          snapshots: snapshots('ce', snapCount, Date.UTC(2024, 7, 1), ceTabs, true),
        }),
      ],
    },
  ],
  activeAccount: uuid('ceacc', 2),
  token: fakeToken,
});

const settingNext = {
  lowConfidencePricing: true, autoSnapshotting: true, isHardwareAccelerationEnabled: true, releaseChannel: 'latest',
  priceThreshold: 2, totalPriceThreshold: 10, showPriceInExalt: true, autoSnapshotInterval: 1_800_000, uiScale: 104,
  logPath: 'C:/Program Files (x86)/Grinding Gear Games/Path of Exile/logs/Client.txt', appExitAction: 'exit',
};
const settingCe = {
  lowConfidencePricing: false, autoSnapshotting: true, isHardwareAccelerationEnabled: true, releaseChannel: 'latest',
  priceThreshold: 0.5, totalPriceThreshold: 0, currency: 'exalt', autoSnapshotInterval: 120_000, uiScale: 87,
  logPath: '/home/foo/Client.txt', appExitAction: 'minimize-to-tray',
};
const uiState = (span, msg) => ({
  userId: uuid('user', 1), sidenavOpen: true, toolbarTourOpen: false, showItemTableFilter: false,
  netWorthChartExpanded: true, tabChartExpanded: false, netWorthItemsExpanded: true, chartTimeSpan: span,
  bulkSellAskingPrice: 0, bulkSellAskingPricePercentage: 100, bulkSellGeneratedMessage: msg, bulkSellGeneratingImage: false,
});
// Custom prices of 0 and missing are kept in legacy but never applied.
const customPrice = {
  customLeaguePrices: [{
    leagueId: 'Settlers',
    prices: [
      { name: 'Divine Orb', icon: 'https://web.poecdn.com/divine.png', frameType: 5, calculated: 140, count: 1, customPrice: 150.5 },
      { name: 'Mirror of Kalandra', icon: 'https://web.poecdn.com/mirror.png', frameType: 5, calculated: 90000, count: 1, customPrice: 0 },
      { name: 'Exalted Orb', icon: 'https://web.poecdn.com/exalt.png', frameType: 5, calculated: 12, count: 1 },
      { name: 'Headhunter', icon: 'https://web.poecdn.com/hh.png', frameType: 3, variant: '', links: 0, quality: 20, level: 0, ilvl: 84,
        tier: 0, corrupted: true, elder: false, shaper: false, calculated: 1200, count: 1, customPrice: 1500 },
    ],
  }],
};

const json = (v) => JSON.stringify(v);

// ---------------------------------------------------------------- sets

function save(set, appDir, home, dump) {
  const src = join(home, '.config', appDir);
  const dst = join(out, set, appDir);
  rmSync(dst, { recursive: true, force: true });
  cpSync(join(src, 'IndexedDB'), join(dst, 'IndexedDB'), { recursive: true });
  cpSync(join(src, 'local-settings.json'), join(dst, 'local-settings.json'));
  const exp = join(out, 'expected', set, appDir);
  rmSync(exp, { recursive: true, force: true });
  mkdirSync(exp, { recursive: true });
  for (const [k, v] of Object.entries(dump)) writeFileSync(join(exp, `${k}.json`), v);
  console.log(set, appDir, Object.fromEntries(Object.entries(dump).map(([k, v]) => [k, v.length])));
}

rmSync(work, { recursive: true, force: true });

// both/exilence-next-app: Exilence Next 1.2.2.
{
  const home = join(work, 'next');
  // Session 1: first launch (the app writes `migration` and local-settings.json), seed, then
  // overwrite `account` around a flush to `.ldb` with tombstones.
  await session(nextRoot, 'exilence-next-app', home, 9411, async (app) => {
    await app.put([['account', json(nextAccount(5))], ['uiState', json(uiState('1 month', ''))], ['customPrice', json({ customLeaguePrices: [] })]]);
    await app.padAndDelete();
    await app.put([['account', json(nextAccount(20))]]);
  });
  // Session 2: the app hydrates and re-writes its stores; then a settings record from before
  // `currency` existed (Exilence Next ≤ 1.1.8 had `showPriceInExalt`).
  const dump = await session(nextRoot, 'exilence-next-app', home, 9412, async (app) => {
    await app.put([['setting', json(settingNext)]]);
    await sleep(1000);
    return app.dump();
  });
  save('both', 'exilence-next-app', home, dump);
}

// both/exilence-ce-app: CE v1.3.0.
{
  const home = join(work, 'ce');
  await session(ceRoot, 'exilence-ce-app', home, 9421, async (app) => {
    await app.put([['account', json(ceAccount(3))], ['setting', json(settingCe)], ['uiState', json(uiState('1 week', 'WTB ✓'))], ['customPrice', json(customPrice)]]);
    await app.padAndDelete();
    await app.put([['account', json(ceAccount(18))]], ['league']);
    await app.ipc('app-exit-action', 'exit');
  });
  const dump = await session(ceRoot, 'exilence-ce-app', home, 9422, async (app) => {
    await sleep(1000);
    return app.dump();
  });
  save('both', 'exilence-ce-app', home, dump);
}

// old-migration/exilence-next-app: pre-1.0 Exilence Next data (`migration.current` 2), which
// legacy wipes on its next launch; written in the last session so no launch migrates it.
{
  const home = join(work, 'old');
  const dump = await session(nextRoot, 'exilence-next-app', home, 9431, async (app) => {
    await app.put([['migration', json({ current: 2 })], ['account', json(nextAccount(2))]]);
    await sleep(1000);
    return app.dump();
  });
  save('old-migration', 'exilence-next-app', home, dump);
}

console.log(readFileSync(join(out, 'both', 'exilence-ce-app', 'local-settings.json'), 'utf8'));
