// The site's pages, drawn in the browser from the JSON API. No framework and
// no build step: one file, compiled into the server binary.
(() => {
  "use strict";

  const app = document.getElementById("app");
  const CLASSES = {
    11: "Gladiator", 12: "Templar", 13: "Assassin", 14: "Ranger", 15: "Sorcerer",
    16: "Spiritmaster", 17: "Cleric", 18: "Chanter", 19: "Brawler",
  };
  const CLASS_COLORS = {
    11: "#e0a458", 12: "#6ea8ff", 13: "#c86bd8", 14: "#7fcf6a", 15: "#ef6b6b",
    16: "#4fd1c5", 17: "#f3e27a", 18: "#9b7bff", 19: "#f08a4b",
  };
  let session = { user: null, siteName: "A2 Tools", openRegistration: true };
  let servers = {};
  let dungeons = {};
  let bossList = null;

  // ── helpers ────────────────────────────────────────────────────────────
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const fmtInt = (n) => Math.round(Number(n) || 0).toLocaleString();
  const fmtShort = (n) => {
    n = Number(n) || 0;
    const a = Math.abs(n);
    if (a >= 1e9) return (n / 1e9).toFixed(2) + "B";
    if (a >= 1e6) return (n / 1e6).toFixed(2) + "M";
    if (a >= 1e4) return (n / 1e3).toFixed(1) + "K";
    return fmtInt(n);
  };
  const fmtDur = (ms) => {
    const s = Math.max(0, Math.round((Number(ms) || 0) / 1000));
    const m = Math.floor(s / 60);
    return `${m}:${String(s % 60).padStart(2, "0")}`;
  };
  const fmtPct = (x) => `${((Number(x) || 0) * 100).toFixed(1)}%`;
  const ago = (secs) => {
    const d = Date.now() / 1000 - Number(secs);
    if (d < 60) return "just now";
    if (d < 3600) return `${Math.floor(d / 60)}m ago`;
    if (d < 86400) return `${Math.floor(d / 3600)}h ago`;
    if (d < 86400 * 30) return `${Math.floor(d / 86400)}d ago`;
    return new Date(secs * 1000).toLocaleDateString();
  };
  const dateTime = (ms) => (ms ? new Date(ms).toLocaleString() : "");
  const className = (id) => CLASSES[id] || "Unknown";
  const classIcon = (id, size) => (CLASSES[id] ? `<img src="/static/class/${id}.png" alt="${esc(className(id))}" title="${esc(className(id))}"${size ? ` width="${size}" height="${size}"` : ""} />` : "");
  const classCell = (id, name) => `<span class="cls">${classIcon(id)}${esc(name ?? className(id))}</span>`;
  const serverName = (id) => servers[id]?.en || (id ? `Server ${id}` : "");
  const dungeonName = (id) => {
    const d = dungeons[id];
    if (!d) return "";
    return d.difficulty ? `${d.name} (${d.difficulty})` : d.name;
  };
  const params = () => new URLSearchParams(location.search);

  async function api(path, opts = {}) {
    const init = { credentials: "same-origin", ...opts, headers: { ...(opts.headers || {}) } };
    if (opts.json !== undefined) {
      init.method = init.method || "POST";
      init.headers["content-type"] = "application/json";
      init.body = JSON.stringify(opts.json);
    }
    const res = await fetch(path, init);
    let data = null;
    try { data = await res.json(); } catch { /* not json */ }
    if (!res.ok) {
      const err = new Error(data?.message || `Request failed (${res.status})`);
      err.status = res.status;
      err.code = data?.error;
      throw err;
    }
    return data;
  }

  const setTitle = (t) => { document.title = t ? `${t} · ${session.siteName}` : session.siteName; };
  const navigate = (url) => { history.pushState({}, "", url); route(); };
  const loading = () => { app.innerHTML = `<p class="muted">Loading…</p>`; };
  const fail = (e) => { app.innerHTML = `<div class="panel"><p class="error">${esc(e.message || e)}</p></div>`; };

  document.addEventListener("click", (e) => {
    const a = e.target.closest("a[data-link]");
    if (!a || e.ctrlKey || e.metaKey || e.shiftKey || e.button !== 0) return;
    e.preventDefault();
    document.querySelector(".nav")?.classList.remove("open");
    navigate(a.getAttribute("href"));
  });
  window.addEventListener("popstate", route);
  document.querySelector(".navToggle")?.addEventListener("click", (e) => {
    const nav = document.querySelector(".nav");
    nav.classList.toggle("open");
    e.currentTarget.setAttribute("aria-expanded", nav.classList.contains("open"));
  });

  function renderNav() {
    const el = document.querySelector(".navUser");
    if (session.user) {
      el.innerHTML = `
        ${session.user.isAdmin ? `<a href="/admin" data-link>Admin</a>` : ""}
        <a href="/account" data-link>${esc(session.user.displayName)}</a>`;
    } else {
      el.innerHTML = `<a href="/login" data-link>Sign in</a>${session.openRegistration ? `<a href="/register" data-link>Create account</a>` : ""}`;
    }
    document.querySelectorAll(".nav a").forEach((a) => {
      const href = a.getAttribute("href");
      a.classList.toggle("active", href !== "/" && location.pathname.startsWith(href));
    });
  }

  async function loadSession() {
    try { session = await api("/api/auth/session"); } catch { /* offline */ }
    document.querySelectorAll(".brandName").forEach((el) => (el.textContent = session.siteName));
    renderNav();
  }

  async function loadMeta() {
    try {
      const [s, d] = await Promise.all([fetch("/static/servers.json").then((r) => r.json()), fetch("/static/dungeons.json").then((r) => r.json())]);
      servers = s.servers || {};
      dungeons = d || {};
    } catch { /* names fall back to ids */ }
  }

  async function bosses() {
    if (!bossList) bossList = (await api("/api/bosses")).bosses;
    return bossList;
  }

  function bossSelect(list, selected, { any = true } = {}) {
    const opts = list.map((b) => `<option value="${b.mobCode}"${String(b.mobCode) === String(selected) ? " selected" : ""}>${esc(b.bossName || `Boss ${b.mobCode}`)}${b.dungeonId && dungeonName(b.dungeonId) ? ` — ${esc(dungeonName(b.dungeonId))}` : ""}</option>`);
    return `<select name="boss">${any ? `<option value="">All bosses</option>` : ""}${opts.join("")}</select>`;
  }

  function classSelect(selected) {
    return `<select name="job"><option value="">All classes</option>${Object.entries(CLASSES).map(([id, n]) => `<option value="${id}"${String(id) === String(selected) ? " selected" : ""}>${n}</option>`).join("")}</select>`;
  }

  function daysSelect(selected) {
    const opts = [["", "All time"], ["7", "Last 7 days"], ["30", "Last 30 days"], ["90", "Last 90 days"]];
    return `<select name="days">${opts.map(([v, n]) => `<option value="${v}"${v === String(selected ?? "") ? " selected" : ""}>${n}</option>`).join("")}</select>`;
  }

  function bindFilters(root, base) {
    root.querySelectorAll(".filters select, .filters input[type=checkbox]").forEach((el) => {
      el.addEventListener("change", () => {
        const q = new URLSearchParams();
        root.querySelectorAll(".filters select, .filters input[type=checkbox], .filters input[type=hidden]").forEach((f) => {
          if (f.type === "checkbox") { if (f.checked) q.set(f.name, "1"); }
          else if (f.value) q.set(f.name, f.value);
        });
        navigate(`${base}${q.toString() ? "?" + q : ""}`);
      });
    });
  }

  function logRow(l) {
    const players = (l.players || []);
    const top = players[0];
    const flags = [
      l.killed ? `<span class="badge good">Kill</span>` : `<span class="badge">Wipe</span>`,
      l.ranked ? "" : `<span class="badge" title="Not counted for leaderboards">Unranked</span>`,
      l.visibility !== "public" ? `<span class="badge warn">${esc(l.visibility)}</span>` : "",
    ].join(" ");
    return `<tr class="click" data-href="/logs/${esc(l.id)}">
      <td><a href="/logs/${esc(l.id)}" data-link><b>${esc(l.bossName || "Unknown boss")}</b></a>
        <div class="small muted">${esc(dungeonName(l.dungeonId))}${l.serverId ? ` · ${esc(serverName(l.serverId))}` : ""}</div></td>
      <td class="num">${fmtDur(l.durationMs)}</td>
      <td><span class="party">${players.map((p) => classIcon(p.jobId)).join("")}</span></td>
      <td class="num">${top ? `${fmtShort(top.dps)}<div class="small muted">${esc(top.name)}</div>` : ""}</td>
      <td>${flags}</td>
      <td class="muted small">${esc(l.uploader)}<br>${ago(l.createdAt)}</td>
    </tr>`;
  }

  function logTable(logs) {
    if (!logs.length) return `<div class="empty">No logs yet.</div>`;
    return `<div class="tableWrap"><table>
      <thead><tr><th>Boss</th><th class="num">Time</th><th>Party</th><th class="num">Top DPS</th><th></th><th>Uploaded</th></tr></thead>
      <tbody>${logs.map(logRow).join("")}</tbody></table></div>`;
  }

  function bindRows(root) {
    root.querySelectorAll("tr[data-href]").forEach((tr) => {
      tr.addEventListener("click", (e) => {
        if (e.target.closest("a")) return;
        navigate(tr.dataset.href);
      });
    });
  }

  // ── pages ──────────────────────────────────────────────────────────────
  async function home() {
    setTitle("");
    const o = await api("/api/overview");
    app.innerHTML = `
      <section class="hero">
        <h1>${esc(session.siteName)}</h1>
        <p>Fight logs, leaderboards, class stats and field boss timers for AION 2, from the A2Tools DPS Meter.
        Every uploaded fight is re-derived from its recorded packets by the meter's own parser, so the numbers are what the game sent.</p>
        <p><a class="btn primary" href="/download" data-link>Connect your meter</a>
        <a class="btn" href="/logs" data-link>Browse logs</a></p>
      </section>
      <div class="grid cols3">
        <div class="panel stat"><span class="muted">Public logs</span><b>${fmtInt(o.logs)}</b></div>
        <div class="panel stat"><span class="muted">Players logged</span><b>${fmtInt(o.players)}</b></div>
        <div class="panel stat"><span class="muted">Accounts</span><b>${fmtInt(o.users)}</b></div>
      </div>
      <h2>Recent logs</h2>
      <div class="panel" id="recent"></div>`;
    const recent = document.getElementById("recent");
    recent.innerHTML = logTable(o.recent);
    bindRows(recent);
  }

  async function logsPage() {
    setTitle("Logs");
    const q = params();
    const mine = q.get("mine") === "1";
    const page = Number(q.get("page") || 0);
    const [list, data] = await Promise.all([bosses(), api(`/api/logs?${new URLSearchParams([...q.entries()].map(([k, v]) => [k, k === "mine" ? "true" : v]))}`)]);
    app.innerHTML = `
      <div class="pageHead"><div><h1>Logs</h1><div class="muted">${fmtInt(data.total)} ${mine ? "of your logs" : "public logs"}</div></div></div>
      <div class="filters">
        ${bossSelect(list, q.get("boss"))}
        ${classSelect(q.get("job"))}
        ${session.user ? `<label class="muted"><input type="checkbox" name="mine"${mine ? " checked" : ""}> Only mine</label>` : ""}
      </div>
      <div class="panel" id="list">${logTable(data.logs)}</div>
      <div class="pager">
        ${page > 0 ? `<button data-page="${page - 1}">← Newer</button>` : ""}
        <span class="muted small">Page ${page + 1} of ${Math.max(1, Math.ceil(data.total / data.pageSize))}</span>
        ${(page + 1) * data.pageSize < data.total ? `<button data-page="${page + 1}">Older →</button>` : ""}
      </div>`;
    bindFilters(app, "/logs");
    bindRows(app);
    app.querySelectorAll("[data-page]").forEach((b) => b.addEventListener("click", () => {
      const nq = params(); nq.set("page", b.dataset.page); navigate(`/logs?${nq}`);
    }));
  }

  // One fight: what the meter's Details window shows.
  async function logPage(id) {
    const log = await api(`/api/logs/${encodeURIComponent(id)}`);
    const fight = log.fight || {};
    const rec = fight.record || {};
    const details = rec.details || {};
    const actors = rec.actors || [];
    const byId = new Map(actors.map((a) => [a.actorId, a]));
    setTitle(rec.bossName || "Log");

    const durS = Math.max(0.001, (Number(rec.durationMs) || 0) / 1000);
    const sum = (list) => {
      const m = new Map();
      for (const s of list || []) {
        const e = m.get(s.actorId) || { dmg: 0, hits: 0, crit: 0, back: 0, perfect: 0, double: 0, parry: 0, skills: [] };
        const hits = Number(s.time) || 0;
        e.dmg += Number(s.dmg) || 0;
        e.hits += hits;
        e.crit += Number(s.crit) || 0;
        e.back += Number(s.back) || 0;
        e.perfect += Number(s.perfect) || 0;
        e.double += Number(s.double) || 0;
        e.parry += Number(s.parry) || 0;
        e.skills.push(s);
        m.set(s.actorId, e);
      }
      return m;
    };
    const dmg = sum(details.skills);
    const heal = sum(details.healSkills);
    const totalDmg = [...dmg.values()].reduce((a, e) => a + e.dmg, 0) || 1;
    const rows = actors
      .map((a) => ({ a, d: dmg.get(a.actorId) || { dmg: 0, hits: 0, crit: 0, back: 0, perfect: 0, double: 0, skills: [] }, h: heal.get(a.actorId) }))
      .sort((x, y) => y.d.dmg - x.d.dmg);
    const maxDmg = rows[0]?.d.dmg || 1;
    const uploaderId = log.uploaderActor ?? rows.find((r) => !String(r.a.nickname).includes("*"))?.a.actorId;
    const deaths = details.deaths || [];
    const checks = fight.checks || {};

    app.innerHTML = `
      <div class="pageHead">
        <div>
          <h1>${esc(rec.bossName || "Unknown boss")}</h1>
          <div class="muted">${esc(dungeonName(rec.dungeonId))}${rec.serverId ? ` · ${esc(serverName(rec.serverId))}` : ""} · ${dateTime(log.fightStartMs || rec.startTimeMs)} · uploaded by ${esc(log.uploader)}</div>
        </div>
        <div class="filters" style="margin:0">
          ${log.killed ? `<span class="badge good">Kill</span>` : `<span class="badge">Not a kill</span>`}
          ${log.ranked ? `<span class="badge acc">Ranked</span>` : `<span class="badge" title="Leaderboards count kills of a boss from a complete recording">Unranked</span>`}
          <button id="copyLink">Copy link</button>
        </div>
      </div>
      <div class="grid cols3">
        <div class="panel stat"><span class="muted">Duration</span><b>${fmtDur(rec.durationMs)}</b></div>
        <div class="panel stat"><span class="muted">Party damage</span><b>${fmtShort(fight.totalDamage ?? rec.totalDamage)}</b></div>
        <div class="panel stat"><span class="muted">Party DPS</span><b>${fmtShort((fight.totalDamage ?? rec.totalDamage) / durS)}</b></div>
      </div>
      <div class="tabs" role="tablist">
        <button class="on" data-tab="damage">Damage</button>
        <button data-tab="chart">DPS over time</button>
        <button data-tab="healing">Healing</button>
        <button data-tab="deaths">Deaths (${deaths.length})</button>
        <button data-tab="info">About this log</button>
      </div>
      <div id="tab"></div>`;

    const tab = document.getElementById("tab");
    document.getElementById("copyLink").addEventListener("click", async (e) => {
      try { await navigator.clipboard.writeText(location.href); e.target.textContent = "Copied"; } catch { /* no clipboard */ }
    });

    let selected = null;
    const skillTable = (actorId) => {
      const e = dmg.get(actorId);
      if (!e) return `<div class="empty">No damage.</div>`;
      const merged = new Map();
      for (const s of e.skills) {
        const key = `${s.name || s.code}${s.isDot ? " (DoT)" : ""}`;
        const m = merged.get(key) || { name: key, dmg: 0, hits: 0, crit: 0, back: 0, perfect: 0, double: 0, max: 0, min: 0 };
        m.dmg += Number(s.dmg) || 0;
        m.hits += Number(s.time) || 0;
        m.crit += Number(s.crit) || 0;
        m.back += Number(s.back) || 0;
        m.perfect += Number(s.perfect) || 0;
        m.double += Number(s.double) || 0;
        m.max = Math.max(m.max, Number(s.maxDmg) || 0);
        const mn = Number(s.minDmg) || 0;
        if (mn > 0) m.min = m.min ? Math.min(m.min, mn) : mn;
        merged.set(key, m);
      }
      const list = [...merged.values()].sort((a, b) => b.dmg - a.dmg);
      const top = list[0]?.dmg || 1;
      const pct = (n, d) => (d ? fmtPct(n / d) : "—");
      return `<div class="tableWrap"><table>
        <thead><tr><th>Skill</th><th>Damage</th><th class="num">Share</th><th class="num">Hits</th><th class="num">Crit</th><th class="num">Back</th><th class="num">Perfect</th><th class="num">Double</th><th class="num">Max hit</th></tr></thead>
        <tbody>${list.map((m) => `<tr>
          <td>${esc(m.name)}</td>
          <td class="barCell"><div class="bar" style="width:${(m.dmg / top) * 100}%"></div><span>${fmtInt(m.dmg)}</span></td>
          <td class="num">${pct(m.dmg, e.dmg)}</td>
          <td class="num">${fmtInt(m.hits)}</td>
          <td class="num">${pct(m.crit, m.hits)}</td>
          <td class="num">${pct(m.back, m.hits)}</td>
          <td class="num">${pct(m.perfect, m.hits)}</td>
          <td class="num">${pct(m.double, m.hits)}</td>
          <td class="num">${fmtInt(m.max)}</td></tr>`).join("")}</tbody></table></div>`;
    };

    const damageTab = () => {
      tab.innerHTML = `<div class="panel"><div class="tableWrap"><table>
        <thead><tr><th>Player</th><th>Damage</th><th class="num">DPS</th><th class="num">Share</th><th class="num">Crit</th><th class="num">Back</th><th class="num">Perfect</th></tr></thead>
        <tbody>${rows.map(({ a, d }) => `<tr class="click${selected === a.actorId ? " sel" : ""}" data-actor="${a.actorId}">
          <td>${classCell(a.jobId, a.nickname)}</td>
          <td class="barCell"><div class="bar${a.actorId === uploaderId ? " me" : ""}" style="width:${(d.dmg / maxDmg) * 100}%"></div><span>${fmtInt(d.dmg)}</span></td>
          <td class="num">${fmtShort(d.dmg / durS)}</td>
          <td class="num">${fmtPct(d.dmg / totalDmg)}</td>
          <td class="num">${d.hits ? fmtPct(d.crit / d.hits) : "—"}</td>
          <td class="num">${d.hits ? fmtPct(d.back / d.hits) : "—"}</td>
          <td class="num">${d.hits ? fmtPct(d.perfect / d.hits) : "—"}</td>
        </tr>`).join("")}</tbody></table></div>
        <p class="muted small">Select a player for their skills. Other players' names are masked; the uploader's is not.</p></div>
        ${selected != null ? `<div class="panel"><h3>${classCell(byId.get(selected)?.jobId, byId.get(selected)?.nickname)}</h3>${skillTable(selected)}</div>` : ""}`;
      tab.querySelectorAll("tr[data-actor]").forEach((tr) => tr.addEventListener("click", () => {
        const id = Number(tr.dataset.actor);
        selected = selected === id ? null : id;
        damageTab();
      }));
    };

    const chartTab = () => {
      const hits = [];
      for (const s of details.skills || []) {
        const ts = Array.isArray(s.hitTimestamps) ? s.hitTimestamps : [];
        const per = ts.length ? (Number(s.dmg) || 0) / ts.length : 0;
        for (const t of ts) hits.push({ a: s.actorId, t: Number(t), d: per });
      }
      if (!hits.length) { tab.innerHTML = `<div class="panel empty">This fight has no hit timing.</div>`; return; }
      const t0 = Math.min(...hits.map((h) => h.t));
      const span = Math.max(Number(rec.durationMs) || 0, Math.max(...hits.map((h) => h.t)) - t0, 1000);
      const windowMs = Math.max(5000, Math.min(15000, span * 0.1));
      const N = 120;
      const top = rows.slice(0, 8).filter((r) => r.d.dmg > 0);
      const series = top.map(({ a }) => {
        const mine = hits.filter((h) => h.a === a.actorId).sort((x, y) => x.t - y.t);
        const pts = [];
        for (let i = 0; i <= N; i++) {
          const t = t0 + (span * i) / N;
          let total = 0;
          for (const h of mine) { if (h.t > t) break; if (h.t > t - windowMs) total += h.d; }
          pts.push(total / (windowMs / 1000));
        }
        return { a, pts };
      });
      const maxY = Math.max(1, ...series.flatMap((s) => s.pts));
      const W = 1000, H = 240, L = 56, R = 10, T = 10, B = 24;
      const x = (i) => L + ((W - L - R) * i) / N;
      const y = (v) => T + (H - T - B) * (1 - v / maxY);
      const ticks = [0, 0.25, 0.5, 0.75, 1].map((f) => maxY * f);
      const palette = ["#6ea8ff", "#e5b94e", "#4cc38a", "#ef6b6b", "#9b7bff", "#4fd1c5", "#f08a4b", "#c86bd8"];
      const color = (s, i) => (series.filter((o) => o.a.jobId === s.a.jobId).length === 1 && CLASS_COLORS[s.a.jobId]) || palette[i % palette.length];
      tab.innerHTML = `<div class="panel">
        <svg class="chart" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" role="img" aria-label="DPS over time">
          <g class="grid">${ticks.map((v) => `<line x1="${L}" x2="${W - R}" y1="${y(v)}" y2="${y(v)}"/>`).join("")}</g>
          ${ticks.map((v) => `<text x="${L - 6}" y="${y(v) + 4}" text-anchor="end">${fmtShort(v)}</text>`).join("")}
          ${[0, 0.25, 0.5, 0.75, 1].map((f) => `<text x="${x(N * f)}" y="${H - 6}" text-anchor="middle">${fmtDur(span * f)}</text>`).join("")}
          ${series.map((s, i) => `<polyline fill="none" stroke="${color(s, i)}" stroke-width="2" vector-effect="non-scaling-stroke" points="${s.pts.map((v, j) => `${x(j).toFixed(1)},${y(v).toFixed(1)}`).join(" ")}"/>`).join("")}
        </svg>
        <div class="legend">${series.map((s, i) => `<span><i style="background:${color(s, i)}"></i>${esc(s.a.nickname)}</span>`).join("")}</div>
        <p class="muted small">Damage per second over a rolling ${Math.round(windowMs / 1000)} s window.</p></div>`;
    };

    const healingTab = () => {
      const list = rows.filter((r) => r.h && r.h.dmg > 0).sort((x, y) => y.h.dmg - x.h.dmg);
      if (!list.length) { tab.innerHTML = `<div class="panel empty">No healing recorded.</div>`; return; }
      const top = list[0].h.dmg || 1;
      tab.innerHTML = `<div class="panel"><div class="tableWrap"><table>
        <thead><tr><th>Player</th><th>Healing</th><th class="num">HPS</th><th class="num">Casts</th></tr></thead>
        <tbody>${list.map(({ a, h }) => `<tr><td>${classCell(a.jobId, a.nickname)}</td>
          <td class="barCell"><div class="bar" style="width:${(h.dmg / top) * 100}%"></div><span>${fmtInt(h.dmg)}</span></td>
          <td class="num">${fmtShort(h.dmg / durS)}</td><td class="num">${fmtInt(h.hits)}</td></tr>`).join("")}</tbody></table></div></div>`;
    };

    const deathsTab = () => {
      if (!deaths.length) { tab.innerHTML = `<div class="panel empty">Nobody died.</div>`; return; }
      const start = Number(details.startTime) || Math.min(...deaths.map((d) => d.tsMs));
      tab.innerHTML = `<div class="panel"><div class="tableWrap"><table>
        <thead><tr><th>Player</th><th class="num">Died at</th><th class="num">Revived</th></tr></thead>
        <tbody>${deaths.map((d) => { const a = byId.get(d.actorId); return `<tr>
          <td>${a ? classCell(a.jobId, a.nickname) : esc(d.actorId)}</td>
          <td class="num">${fmtDur(d.tsMs - start)}</td>
          <td class="num">${d.revivedMs ? fmtDur(d.revivedMs - start) : "—"}</td></tr>`; }).join("")}</tbody></table></div></div>`;
    };

    const infoTab = () => {
      tab.innerHTML = `<div class="panel"><dl class="kv">
        <dt>Parser</dt><dd>${esc(fight.parserVersion || log.parserVersion)}</dd>
        <dt>Boss max HP</dt><dd>${fmtInt(checks.maxHp)}</dd>
        <dt>Damage on the boss</dt><dd>${fmtInt(checks.damage)}${checks.maxHp ? ` (${fmtPct(checks.damage / checks.maxHp)} of its HP)` : ""}</dd>
        <dt>Packets</dt><dd>${fmtInt(checks.records)} records, ${fmtInt(checks.bundles)} bundles, ${fmtInt(checks.lifted)} lifted</dd>
        <dt>Visibility</dt><dd>${esc(log.visibility)}</dd>
        </dl>
        <p class="muted small">The numbers on this page were derived on the server from the fight's recorded packets with the meter's own parser, not sent by the uploader.
        <a href="/api/logs/${esc(log.id)}/slice">Download the recording</a> to check it with <code>a2t-derive</code>.</p>
        ${log.mine || log.canDelete ? `<h3>Manage</h3><div class="filters">
          ${log.mine ? `<select id="vis">${["public", "unlisted", "private"].map((v) => `<option${v === log.visibility ? " selected" : ""}>${v}</option>`).join("")}</select>` : ""}
          <button class="danger" id="del">Delete log</button><span id="manageMsg" class="small"></span></div>` : ""}</div>`;
      document.getElementById("vis")?.addEventListener("change", async (e) => {
        const msg = document.getElementById("manageMsg");
        try { await api(`/api/logs/${log.id}`, { method: "PATCH", json: { visibility: e.target.value } }); log.visibility = e.target.value; msg.className = "ok small"; msg.textContent = "Saved"; }
        catch (err) { msg.className = "error small"; msg.textContent = err.message; }
      });
      document.getElementById("del")?.addEventListener("click", async () => {
        if (!confirm("Delete this log for good?")) return;
        try { await api(`/api/logs/${log.id}`, { method: "DELETE" }); navigate("/logs?mine=1"); }
        catch (err) { document.getElementById("manageMsg").textContent = err.message; }
      });
    };

    const tabs = { damage: damageTab, chart: chartTab, healing: healingTab, deaths: deathsTab, info: infoTab };
    app.querySelectorAll(".tabs button").forEach((b) => b.addEventListener("click", () => {
      app.querySelectorAll(".tabs button").forEach((o) => o.classList.toggle("on", o === b));
      tabs[b.dataset.tab]();
    }));
    damageTab();
  }

  async function leaderboardsPage() {
    setTitle("Leaderboards");
    const q = params();
    const list = (await bosses()).filter((b) => b.ranked > 0);
    if (!list.length) {
      app.innerHTML = `<h1>Leaderboards</h1><div class="panel empty">No ranked kills yet. Upload a boss kill from the meter to start one.</div>`;
      return;
    }
    const boss = q.get("boss") || String(list[0].mobCode);
    const by = q.get("by") === "speed" ? "speed" : "dps";
    const qs = new URLSearchParams({ boss, by });
    if (q.get("job") && by === "dps") qs.set("job", q.get("job"));
    if (q.get("days")) qs.set("days", q.get("days"));
    const data = await api(`/api/leaderboards?${qs}`);
    const body = by === "speed"
      ? `<table><thead><tr><th>#</th><th class="num">Kill time</th><th>Party</th><th class="num">Party DPS</th><th>Uploaded</th></tr></thead>
         <tbody>${data.rows.map((l, i) => `<tr class="click" data-href="/logs/${esc(l.id)}"><td>${i + 1}</td><td class="num"><b>${fmtDur(l.durationMs)}</b></td>
           <td><span class="party">${(l.players || []).map((p) => classIcon(p.jobId)).join("")}</span></td>
           <td class="num">${fmtShort(l.totalDamage / Math.max(1, l.durationMs / 1000))}</td><td class="muted small">${esc(l.uploader)} · ${ago(l.createdAt)}</td></tr>`).join("")}</tbody></table>`
      : `<table><thead><tr><th>#</th><th>Player</th><th class="num">DPS</th><th class="num">Damage</th><th class="num">Kill time</th><th>Uploaded</th></tr></thead>
         <tbody>${data.rows.map((r, i) => `<tr class="click" data-href="/logs/${esc(r.logId)}"><td>${i + 1}</td><td>${classCell(r.jobId, r.name)}</td>
           <td class="num"><b>${fmtShort(r.dps)}</b></td><td class="num">${fmtShort(r.damage)}</td><td class="num">${fmtDur(r.durationMs)}</td>
           <td class="muted small">${esc(r.uploader)} · ${ago(r.createdAt)}</td></tr>`).join("")}</tbody></table>`;
    app.innerHTML = `
      <div class="pageHead"><div><h1>Leaderboards</h1><div class="muted">Ranked kills from public logs</div></div>
        <div class="seg"><button data-by="dps" class="${by === "dps" ? "on" : ""}">Top DPS</button><button data-by="speed" class="${by === "speed" ? "on" : ""}">Fastest kills</button></div></div>
      <div class="filters">${bossSelect(list, boss, { any: false })}${by === "dps" ? classSelect(q.get("job")) : ""}${daysSelect(q.get("days"))}<input type="hidden" name="by" value="${by}"></div>
      <div class="panel tableWrap">${data.rows.length ? body : `<div class="empty">Nothing here yet.</div>`}</div>`;
    bindFilters(app, "/leaderboards");
    bindRows(app);
    app.querySelectorAll("[data-by]").forEach((b) => b.addEventListener("click", () => {
      const nq = params(); nq.set("by", b.dataset.by); nq.set("boss", boss); navigate(`/leaderboards?${nq}`);
    }));
  }

  async function classesPage() {
    setTitle("Class stats");
    const q = params();
    const list = (await bosses()).filter((b) => b.ranked > 0);
    const qs = new URLSearchParams();
    if (q.get("boss")) qs.set("boss", q.get("boss"));
    if (q.get("days")) qs.set("days", q.get("days"));
    const data = await api(`/api/stats/classes?${qs}`);
    const max = Math.max(1, ...data.classes.map((c) => c.p95));
    app.innerHTML = `
      <div class="pageHead"><div><h1>Class stats</h1><div class="muted">DPS by class across ranked kills in public logs</div></div></div>
      <div class="filters">${bossSelect(list, q.get("boss"))}${daysSelect(q.get("days"))}</div>
      <div class="panel tableWrap">${data.classes.length ? `<table>
        <thead><tr><th>Class</th><th class="num">Players</th><th class="num">Median</th><th class="num">Mean</th><th class="num">Top 5%</th><th class="num">Best</th><th>Middle half (25%–75%)</th></tr></thead>
        <tbody>${data.classes.map((c) => `<tr><td>${classCell(c.jobId)}</td><td class="num">${fmtInt(c.count)}</td>
          <td class="num"><b>${fmtShort(c.median)}</b></td><td class="num">${fmtShort(c.mean)}</td><td class="num">${fmtShort(c.p95)}</td><td class="num">${fmtShort(c.max)}</td>
          <td><div class="range" title="${fmtShort(c.p25)} – ${fmtShort(c.p75)}"><div class="track"></div>
            <div class="iqr" style="left:${(c.p25 / max) * 100}%;width:${Math.max(0.5, ((c.p75 - c.p25) / max) * 100)}%"></div>
            <div class="med" style="left:${(c.median / max) * 100}%"></div></div></td></tr>`).join("")}</tbody></table>` : `<div class="empty">No ranked kills yet.</div>`}</div>`;
    bindFilters(app, "/classes");
  }

  let timerTick = null;
  async function timersPage() {
    setTitle("Boss timers");
    const [data, names] = await Promise.all([api("/api/boss-timers"), api("/api/boss-timers/bosses")]);
    const nameOf = new Map(names.bosses.map((b) => [b.id, b.name]));
    const list = data.servers.filter((s) => Object.keys(s.bosses || {}).length).sort((a, b) => a.server - b.server);
    const q = params();
    if (!list.length) {
      app.innerHTML = `<h1>Boss timers</h1><div class="panel empty">No reports yet. Meters with "Share field boss timers" on report what their map shows.</div>`;
      return;
    }
    const server = Number(q.get("server")) || list[0].server;
    const cur = list.find((s) => s.server === server) || list[0];
    const draw = () => {
      const now = Date.now() / 1000;
      const entries = Object.entries(cur.bosses).map(([id, e]) => ({ id, e })).sort((a, b) => {
        const ka = a.e.state === "up" ? -1e12 + (a.e.upSince || 0) : a.e.nextSpawn || 9e12;
        const kb = b.e.state === "up" ? -1e12 + (b.e.upSince || 0) : b.e.nextSpawn || 9e12;
        return ka - kb;
      });
      const when = (e) => {
        if (e.state === "up") return `<span class="badge good">Up</span> ${e.upSince ? `<span class="muted small">since ${ago(e.upSince)}</span>` : ""}`;
        if (e.nextSpawn) {
          const left = e.nextSpawn - now;
          return left > 0 ? `<b>${fmtDur(left * 1000)}</b> <span class="muted small">at ${new Date(e.nextSpawn * 1000).toLocaleTimeString()}</span>` : `<span class="badge warn">Due</span>`;
        }
        return `<span class="badge">Dead</span> ${e.diedAt ? `<span class="muted small">${ago(e.diedAt)}</span>` : ""}`;
      };
      document.getElementById("timers").innerHTML = `<table>
        <thead><tr><th>Boss</th><th>Status</th><th>HP</th><th></th><th>Reported</th></tr></thead>
        <tbody>${entries.map(({ id, e }) => `<tr><td>${esc(nameOf.get(id) || `Boss ${id}`)}</td><td>${when(e)}</td>
          <td>${e.state === "up" && e.hp != null ? `<span class="hpbar"><i style="width:${e.hp}%"></i></span> <span class="small">${Number(e.hp).toFixed(0)}%</span>` : ""}</td>
          <td>${e.confirmed ? `<span class="badge acc" title="Two players agree">Confirmed</span>` : ""}</td>
          <td class="muted small">${ago(e.reportedAt)}</td></tr>`).join("")}</tbody></table>`;
    };
    app.innerHTML = `
      <div class="pageHead"><div><h1>Boss timers</h1><div class="muted">Field bosses as players' meters report them</div></div></div>
      <div class="filters"><select name="server">${list.map((s) => `<option value="${s.server}"${s.server === cur.server ? " selected" : ""}>${esc(serverName(s.server))} (${s.server})</option>`).join("")}</select></div>
      <div class="panel tableWrap" id="timers"></div>`;
    bindFilters(app, "/timers");
    draw();
    clearInterval(timerTick);
    timerTick = setInterval(() => { if (document.getElementById("timers")) draw(); else clearInterval(timerTick); }, 1000);
  }

  function serverUrl() { return location.origin; }

  function downloadPage() {
    setTitle("Download");
    app.innerHTML = `
      <h1>Connect your meter</h1>
      <div class="panel">
        <h3>1. Install the meter</h3>
        <p>Install the A2Tools DPS Meter build that can use your own server, from this site's admin or from the fork's
        <a href="https://github.com/CharlesPicard/A2Tools-DPS-Meter/releases" rel="noopener">releases</a>.
        On Windows it needs <a href="https://npcap.com" rel="noopener">Npcap</a> (WinPcap compatible mode) and runs as administrator.</p>
        <h3>2. Point it at this site</h3>
        <p>Open <b>Settings → A2 Tools Account</b>, and under <b>Server</b> enter:</p>
        <pre>${esc(serverUrl())}</pre>
        <p class="muted small">Or start the meter with the environment variable <code>A2TOOLS_API=${esc(serverUrl())}</code>.</p>
        <h3>3. Sign in</h3>
        <p>Press <b>Sign in</b> in the meter. Your browser opens this site's <a href="/link" data-link>link page</a> with a code; approve it while signed in here.
        ${session.user ? "" : `You need an account: <a href="/register" data-link>create one</a> or <a href="/login" data-link>sign in</a>.`}</p>
        <h3>4. Upload fights</h3>
        <p>Upload a fight from the meter's history, or turn on <b>Upload boss fights automatically</b>. New logs take the visibility set on your <a href="/account" data-link>account</a>.</p>
      </div>`;
  }

  function authForm(kind) {
    const register = kind === "register";
    setTitle(register ? "Create account" : "Sign in");
    const next = params().get("next") || "/";
    if (register && !session.openRegistration && session.user == null) {
      app.innerHTML = `<h1>Create account</h1><div class="panel">This site does not take new accounts. Ask its admin for one.</div>`;
      return;
    }
    app.innerHTML = `
      <h1>${register ? "Create account" : "Sign in"}</h1>
      <div class="panel"><form class="form" id="auth">
        <label>Username<input type="text" name="username" autocomplete="username" required minlength="3" maxlength="32"></label>
        ${register ? `<label>Name shown on logs<input type="text" name="display_name" maxlength="32" placeholder="Same as username"></label>` : ""}
        <label>Password<input type="password" name="password" autocomplete="${register ? "new-password" : "current-password"}" required minlength="${register ? 8 : 1}"></label>
        <button class="primary" type="submit">${register ? "Create account" : "Sign in"}</button>
        <p class="error" id="authErr"></p>
        <p class="muted small">${register ? `Have an account? <a href="/login?next=${encodeURIComponent(next)}" data-link>Sign in</a>` : (session.openRegistration ? `New here? <a href="/register?next=${encodeURIComponent(next)}" data-link>Create an account</a>` : "")}</p>
      </form></div>`;
    document.getElementById("auth").addEventListener("submit", async (e) => {
      e.preventDefault();
      const f = new FormData(e.target);
      try {
        await api(`/api/auth/${register ? "register" : "login"}`, { json: Object.fromEntries(f.entries()) });
        await loadSession();
        bossList = null;
        navigate(next.startsWith("/") && !next.startsWith("//") ? next : "/");
      } catch (err) {
        document.getElementById("authErr").textContent = err.message;
      }
    });
  }

  function requireUser() {
    if (session.user) return true;
    navigate(`/login?next=${encodeURIComponent(location.pathname + location.search)}`);
    return false;
  }

  async function linkPage() {
    setTitle("Connect a meter");
    if (!requireUser()) return;
    const code = (params().get("code") || "").toUpperCase();
    app.innerHTML = `
      <h1>Connect a meter</h1>
      <div class="panel"><form class="form" id="linkForm">
        <label>Code shown in the meter<input type="text" name="code" value="${esc(code)}" placeholder="ABCD-1234" autocomplete="off" required></label>
        <button class="primary" type="submit">Continue</button>
        <p class="error" id="linkErr"></p></form>
        <div id="linkAsk"></div></div>`;
    const ask = async (c) => {
      const box = document.getElementById("linkAsk");
      const err = document.getElementById("linkErr");
      err.textContent = "";
      try {
        const info = await api(`/api/device/lookup?code=${encodeURIComponent(c)}`);
        document.getElementById("linkForm").style.display = "none";
        box.innerHTML = `
          <p><b>${esc(info.label || "A DPS meter")}</b> asks to sign in to <b>${esc(session.siteName)}</b> as <b>${esc(session.user.displayName)}</b>.</p>
          <p class="code">${esc(c)}</p>
          <p class="muted small">Check that this is the code your meter shows. It may then upload fights to your account and read your profile.</p>
          <div class="filters"><button class="primary" id="approve">Allow</button><button id="deny">Deny</button></div>
          <p id="linkDone"></p>`;
        const decide = async (approve) => {
          try {
            await api("/api/device/decide", { json: { code: c, approve } });
            box.innerHTML = approve ? `<p class="ok">Done. The meter is signed in within a few seconds; you can close this tab.</p>` : `<p>Declined. The meter was not signed in.</p>`;
          } catch (e) { document.getElementById("linkDone").innerHTML = `<span class="error">${esc(e.message)}</span>`; }
        };
        document.getElementById("approve").addEventListener("click", () => decide(true));
        document.getElementById("deny").addEventListener("click", () => decide(false));
      } catch (e) { err.textContent = e.message; }
    };
    document.getElementById("linkForm").addEventListener("submit", (e) => {
      e.preventDefault();
      ask(new FormData(e.target).get("code").trim().toUpperCase());
    });
    if (code) ask(code);
  }

  async function accountPage() {
    setTitle("Account");
    if (!requireUser()) return;
    const u = session.user;
    const tokens = (await api("/api/account/tokens")).tokens;
    app.innerHTML = `
      <div class="pageHead"><div><h1>${esc(u.displayName)}</h1><div class="muted">@${esc(u.username)}${u.isAdmin ? " · admin" : ""}${u.supporter ? " · supporter" : ""}</div></div>
        <div class="filters" style="margin:0"><a class="btn" href="/logs?mine=1" data-link>My logs</a><button id="logout">Sign out</button></div></div>
      <div class="panel"><h3>Profile</h3><form class="form" id="profile">
        <label>Name shown on logs<input type="text" name="displayName" value="${esc(u.displayName)}" maxlength="32"></label>
        <label>New logs are<select name="defaultVisibility">${[["public", "Public: listed, and on leaderboards"], ["unlisted", "Unlisted: anyone with the link"], ["private", "Private: only you"]].map(([v, n]) => `<option value="${v}"${v === u.defaultVisibility ? " selected" : ""}>${n}</option>`).join("")}</select></label>
        <button class="primary" type="submit">Save</button><p id="profileMsg" class="small"></p></form></div>
      <div class="panel"><h3>Password</h3><form class="form" id="pw">
        <label>Current password<input type="password" name="currentPassword" autocomplete="current-password" required></label>
        <label>New password<input type="password" name="newPassword" autocomplete="new-password" minlength="8" required></label>
        <button type="submit">Change password</button><p id="pwMsg" class="small"></p></form></div>
      <div class="panel"><h3>Signed-in meters</h3>
        ${tokens.length ? `<div class="tableWrap"><table><thead><tr><th>Meter</th><th>Signed in</th><th>Last used</th><th></th></tr></thead><tbody>
          ${tokens.map((t) => `<tr><td>${esc(t.label)}</td><td class="muted">${ago(t.createdAt)}</td><td class="muted">${t.lastUsed ? ago(t.lastUsed) : "never"}</td>
            <td><button class="danger" data-revoke="${t.id}">Sign out</button></td></tr>`).join("")}</tbody></table></div>` : `<p class="muted">No meter is signed in. See <a href="/download" data-link>how to connect one</a>.</p>`}</div>`;
    const msg = (id, ok, text) => { const el = document.getElementById(id); el.className = `small ${ok ? "ok" : "error"}`; el.textContent = text; };
    document.getElementById("profile").addEventListener("submit", async (e) => {
      e.preventDefault();
      try { await api("/api/account", { method: "PATCH", json: Object.fromEntries(new FormData(e.target).entries()) }); await loadSession(); msg("profileMsg", true, "Saved"); }
      catch (err) { msg("profileMsg", false, err.message); }
    });
    document.getElementById("pw").addEventListener("submit", async (e) => {
      e.preventDefault();
      try { await api("/api/account", { method: "PATCH", json: Object.fromEntries(new FormData(e.target).entries()) }); e.target.reset(); msg("pwMsg", true, "Password changed"); }
      catch (err) { msg("pwMsg", false, err.message); }
    });
    document.getElementById("logout").addEventListener("click", async () => { await api("/api/auth/logout", { method: "POST" }); await loadSession(); navigate("/"); });
    app.querySelectorAll("[data-revoke]").forEach((b) => b.addEventListener("click", async () => {
      await api(`/api/account/tokens/${b.dataset.revoke}`, { method: "DELETE" });
      accountPage();
    }));
  }

  async function adminPage() {
    setTitle("Admin");
    if (!requireUser()) return;
    if (!session.user.isAdmin) { app.innerHTML = `<div class="panel">Admins only.</div>`; return; }
    const [users, dev] = await Promise.all([api("/api/admin/users"), api("/api/admin/dev-logs")]);
    app.innerHTML = `
      <h1>Admin</h1>
      <div class="panel"><h3>Accounts</h3><div class="tableWrap"><table>
        <thead><tr><th>User</th><th class="num">Logs</th><th>Joined</th><th>Admin</th><th>Supporter</th><th></th></tr></thead>
        <tbody>${users.users.map(({ user: u, createdAt, logs }) => `<tr data-user="${u.id}">
          <td>${esc(u.displayName)} <span class="muted small">@${esc(u.username)}</span></td><td class="num">${fmtInt(logs)}</td><td class="muted">${ago(createdAt)}</td>
          <td><input type="checkbox" data-field="isAdmin"${u.isAdmin ? " checked" : ""}${u.id === session.user.id ? " disabled" : ""}></td>
          <td><input type="checkbox" data-field="supporter"${u.supporter ? " checked" : ""}></td>
          <td><button data-act="pw">Set password</button> ${u.id === session.user.id ? "" : `<button class="danger" data-act="del">Delete</button>`}</td></tr>`).join("")}</tbody></table></div>
        <p class="small" id="adminMsg"></p></div>
      <div class="panel"><h3>Parser</h3><p>Re-derive every log made by an older parser from its recording, with this server's parser.</p>
        <button id="rederive">Re-derive logs</button> <span class="small" id="rederiveMsg"></span></div>
      <div class="panel"><h3>Dev logs</h3><p class="muted small">Packet logs players sent with "Send logs to dev". They hold everything the game sent, names included; deleted after ${dev.keepDays} days.</p>
        ${dev.reports.length ? `<div class="tableWrap"><table><thead><tr><th>Code</th><th>From</th><th>Version</th><th>Sent</th><th>Files</th><th></th></tr></thead><tbody>
          ${dev.reports.map((r) => `<tr><td><code>${esc(r.code)}</code></td><td>${esc(r.user || "anonymous")}</td><td>${esc(r.appVersion)}</td><td class="muted">${ago(r.createdAt)}</td>
            <td>${r.files.map((f) => `<a href="/api/admin/dev-logs/${esc(r.code)}/${esc(f)}">${esc(f)}</a>`).join("<br>")}</td>
            <td><button class="danger" data-dev="${esc(r.code)}">Delete</button></td></tr>`).join("")}</tbody></table></div>` : `<p class="muted">None.</p>`}</div>`;
    const msg = (t, ok) => { const el = document.getElementById("adminMsg"); el.className = `small ${ok ? "ok" : "error"}`; el.textContent = t; };
    app.querySelectorAll("tr[data-user]").forEach((tr) => {
      const id = tr.dataset.user;
      tr.querySelectorAll("input[data-field]").forEach((cb) => cb.addEventListener("change", async () => {
        try { await api(`/api/admin/users/${id}`, { method: "PATCH", json: { [cb.dataset.field]: cb.checked } }); msg("Saved", true); }
        catch (e) { msg(e.message); cb.checked = !cb.checked; }
      }));
      tr.querySelector("[data-act=pw]").addEventListener("click", async () => {
        const p = prompt("New password (at least 8 characters)");
        if (!p) return;
        try { await api(`/api/admin/users/${id}`, { method: "PATCH", json: { newPassword: p } }); msg("Password set", true); } catch (e) { msg(e.message); }
      });
      tr.querySelector("[data-act=del]")?.addEventListener("click", async () => {
        if (!confirm("Delete this account and all its logs?")) return;
        try { await api(`/api/admin/users/${id}`, { method: "PATCH", json: { delete: true } }); adminPage(); } catch (e) { msg(e.message); }
      });
    });
    document.getElementById("rederive").addEventListener("click", async (e) => {
      e.target.disabled = true;
      const m = document.getElementById("rederiveMsg");
      m.textContent = "Working…";
      try { const r = await api("/api/admin/rederive", { method: "POST" }); m.textContent = `Parser ${r.parserVersion}: ${r.updated} updated, ${r.failed} failed.`; }
      catch (err) { m.textContent = err.message; }
      e.target.disabled = false;
    });
    app.querySelectorAll("[data-dev]").forEach((b) => b.addEventListener("click", async () => {
      if (!confirm("Delete this report?")) return;
      await api(`/api/admin/dev-logs/${b.dataset.dev}`, { method: "DELETE" });
      adminPage();
    }));
  }

  // ── router ─────────────────────────────────────────────────────────────
  async function route() {
    const path = location.pathname.replace(/\/+$/, "") || "/";
    renderNav();
    window.scrollTo(0, 0);
    loading();
    try {
      if (path === "/") return await home();
      if (path === "/logs") return await logsPage();
      if (path.startsWith("/logs/")) return await logPage(decodeURIComponent(path.slice(6)));
      if (path === "/leaderboards") return await leaderboardsPage();
      if (path === "/classes") return await classesPage();
      if (path === "/timers") return await timersPage();
      if (path === "/download") return downloadPage();
      if (path === "/login") return authForm("login");
      if (path === "/register") return authForm("register");
      if (path === "/link") return await linkPage();
      if (path === "/account") return await accountPage();
      if (path === "/admin") return await adminPage();
      app.innerHTML = `<div class="panel empty">Page not found.</div>`;
    } catch (e) {
      if (e.status === 404) app.innerHTML = `<div class="panel empty">Not found. It may be private or deleted.</div>`;
      else fail(e);
    }
  }

  (async () => {
    await Promise.all([loadSession(), loadMeta()]);
    api("/api/overview").then((o) => { document.querySelector(".parserVersion").textContent = `Parser ${o.parserVersion}`; }).catch(() => {});
    route();
  })();
})();
