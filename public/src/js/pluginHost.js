// A plugin window's host page (src-tauri/src/plugins). The plugin runs in a
// sandboxed frame with an origin of its own and no access to this page or the
// meter; it asks for things by message, and this page passes on only what the
// plugin's manifest permits.
(() => {
  const { invoke, convertFileSrc } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const id = window.__A2_PLUGIN_ID__;
  const stage = document.querySelector(".pluginStage");
  const toast = document.querySelector(".pluginToast");
  let frame = null;
  let manifest = null;
  let ready = false;
  const queue = [];

  const allowed = (permission) => !!manifest?.permissions?.includes(permission);
  const send = (msg) => frame?.contentWindow?.postMessage({ a2: 1, ...msg }, "*");

  // Decoded events are numbered for the meter's run. The page remembers the
  // last one it handed the plugin, so a window opened again (or reloaded)
  // gets each event once: the ones it missed while closed, none twice.
  const CURSOR_KEY = `a2plugin.cursor.${id}`;
  let run = null;
  let delivered = 0;
  const saveCursor = () => {
    try { localStorage.setItem(CURSOR_KEY, JSON.stringify({ run, seq: delivered })); } catch {}
  };

  const deliver = (event, data, extra = {}) => {
    if (event.startsWith("decoded:") && !allowed(`decoder:${event.slice(8)}`)) return;
    if (event === "character" && !allowed("events:character")) return;
    if ((event === "bossMap" || event === "bossLive" || event === "abyss") && !allowed("feed:boss-timers")) return;
    const msg = { event, data, ...extra };
    if (ready) send(msg);
    else queue.push(msg);
  };
  // A numbered event from the meter: skipped if already handed over.
  const deliverLogged = (e) => {
    if (typeof e?.seq !== "number" || e.seq <= delivered) return;
    delivered = e.seq;
    deliver(e.event, e.data, { character: e.character ?? null, at: e.at ?? null });
    if (ready) saveCursor();
  };
  // Events that arrive before the page knows the run and the manifest.
  let early = [];

  let toastTimer = 0;
  const showToast = (text) => {
    toast.textContent = text.slice(0, 200);
    toast.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { toast.hidden = true; }, 4000);
  };

  const calls = {
    ready: () => {
      ready = true;
      for (const m of queue.splice(0)) send(m);
      saveCursor();
      return true;
    },
    "storage.get": ([key]) => invoke("plugin_storage_get", { key: String(key) }),
    // The field boss timers players' meters share, for one server.
    bossTimers: ([server]) => {
      if (!allowed("feed:boss-timers")) throw new Error('the plugin lacks the "feed:boss-timers" permission');
      return invoke("plugin_boss_timers", { server: Number(server) });
    },
    // A server's Abyss siege: the artifacts' holders and the Dimensional
    // Cores (meter 2.0.72).
    abyssSiege: ([server]) => {
      if (!allowed("feed:boss-timers")) throw new Error('the plugin lacks the "feed:boss-timers" permission');
      return invoke("plugin_abyss_siege", { server: Number(server) });
    },
    // Follow some of a server's field bosses: each change comes at once as a
    // "bossLive" event (meter 2.0.68). An empty list stops following.
    "bossTimers.follow": ([server, bosses]) => {
      if (!allowed("feed:boss-timers")) throw new Error('the plugin lacks the "feed:boss-timers" permission');
      return invoke("plugin_boss_follow", { server: Number(server), bosses: (Array.isArray(bosses) ? bosses : []).map(String) });
    },
    "storage.set": ([key, value]) => invoke("plugin_storage_set", { key: String(key), value: String(value) }),
    "window.show": () => {
      if (!allowed("window")) throw new Error('the plugin lacks the "window" permission');
      return invoke("plugin_window", { visible: true });
    },
    // Fit the window to a height of content (an alert, say), its title bar
    // hidden meanwhile; or put its size and title bar back (no height).
    "window.fit": ([height]) => {
      if (!allowed("window")) throw new Error('the plugin lacks the "window" permission');
      const h = Number(height);
      const fitted = Number.isFinite(h) && h > 0;
      document.body.classList.toggle("pluginFitted", fitted);
      return invoke("plugin_window_fit", { height: fitted ? h : null });
    },
    "window.hide": () => {
      if (!allowed("window")) throw new Error('the plugin lacks the "window" permission');
      return invoke("plugin_window", { visible: false });
    },
    // Small windows of the plugin's own, which the player can move and lock
    // (plugin widgets, #77; permission "widgets").
    "widgets.open": ([name, size]) => {
      if (!allowed("widgets")) throw new Error('the plugin lacks the "widgets" permission');
      return invoke("plugin_widget_open", { name: String(name), width: Number(size?.width) || 64, height: Number(size?.height) || 64 });
    },
    "widgets.close": ([name]) => {
      if (!allowed("widgets")) throw new Error('the plugin lacks the "widgets" permission');
      return invoke("plugin_widget_close", { name: String(name) });
    },
    "widgets.send": ([name, data]) => {
      if (!allowed("widgets")) throw new Error('the plugin lacks the "widgets" permission');
      return invoke("plugin_widget_send", { name: String(name), data: data ?? null });
    },
    "widgets.lock": ([locked]) => {
      if (!allowed("widgets")) throw new Error('the plugin lacks the "widgets" permission');
      return invoke("plugin_widgets_lock", { locked: !!locked });
    },
    "widgets.locked": () => {
      if (!allowed("widgets")) throw new Error('the plugin lacks the "widgets" permission');
      return invoke("plugin_widgets_locked");
    },
    "widgets.where": ([name]) => {
      if (!allowed("widgets")) throw new Error('the plugin lacks the "widgets" permission');
      return invoke("plugin_widget_where", { name: String(name) });
    },
    "widgets.place": ([name, at]) => {
      if (!allowed("widgets")) throw new Error('the plugin lacks the "widgets" permission');
      return invoke("plugin_widget_place", { name: String(name), x: Math.round(Number(at?.x)), y: Math.round(Number(at?.y)) });
    },
    notify: ([text]) => {
      if (!allowed("notify")) throw new Error('the plugin lacks the "notify" permission');
      showToast(String(text));
      return true;
    },
    // A link clicked in the plugin's page (the SDK passes on https ones): its
    // frame may not open windows, so the browser opens it. Only plain https
    // addresses, and one a second, so a plugin can't open a stream of tabs.
    openUrl: ([raw]) => {
      const url = new URL(String(raw));
      if (url.protocol !== "https:" || url.username || url.password || url.href.length > 2048) {
        throw new Error("only https links open");
      }
      const now = Date.now();
      if (now - lastLinkAt < 1000) throw new Error("one link a second");
      lastLinkAt = now;
      return invoke("open_url", { url: url.href });
    },
  };
  let lastLinkAt = 0;

  window.addEventListener("message", async (event) => {
    if (!frame || event.source !== frame.contentWindow) return;
    const msg = event.data;
    if (!msg || msg.a2 !== 1 || !msg.id || typeof msg.call !== "string") return;
    const fn = calls[msg.call];
    try {
      if (!fn) throw new Error(`unknown call ${msg.call}`);
      send({ id: msg.id, result: await fn(Array.isArray(msg.args) ? msg.args : []) });
    } catch (e) {
      send({ id: msg.id, error: String(e?.message || e) });
    }
  });

  // The window is frameless: its bar moves it, and the button closes it.
  document.querySelector(".pluginBar").addEventListener("mousedown", (e) => {
    if (e.button !== 0 || e.target.closest("button, input")) return;
    e.preventDefault();
    invoke("start_tool_drag").catch(() => {});
  });
  document.querySelector(".pluginClose").addEventListener("click", () => {
    // Hidden: the plugin keeps running (its widgets and alerts) until it is
    // turned off in Settings, where Open shows it again.
    invoke("plugin_close").catch(() => {});
  });

  // The DPS meter itself, for players who use only plugins: show or hide it
  // (kept across restarts), and Settings, which is otherwise on the meter.
  const meterBtn = document.querySelector(".pluginMeter");
  const showMeterState = (shown) => {
    meterBtn.setAttribute("aria-pressed", String(shown));
    const label = shown ? "Hide the DPS meter" : "Show the DPS meter";
    meterBtn.setAttribute("aria-label", label);
    meterBtn.title = label;
  };
  invoke("meter_shown").then((shown) => showMeterState(shown !== false)).catch(() => {});
  listen("meter-shown", (event) => showMeterState(event.payload !== false));
  meterBtn.addEventListener("click", () => {
    invoke("set_meter_shown", { shown: meterBtn.getAttribute("aria-pressed") !== "true" }).catch(() => {});
  });
  document.querySelector(".pluginSettings").addEventListener("click", () => {
    invoke("open_settings_window").catch(() => {});
  });

  // Panel preferences, per plugin, kept by this page: background opacity, and
  // whether the plugin's controls are folded away ("compact", which the plugin
  // hears as an event and handles itself).
  const PREFS_KEY = `a2plugin.panel.${id}`;
  let panel = { opacity: 100, compact: false };
  try { panel = { ...panel, ...JSON.parse(localStorage.getItem(PREFS_KEY) || "{}") }; } catch {}
  const savePanel = () => { try { localStorage.setItem(PREFS_KEY, JSON.stringify(panel)); } catch {} };
  const opacity = document.querySelector(".pluginOpacity");
  const compact = document.querySelector(".pluginCompact");
  const applyOpacity = () => document.body.style.setProperty("--panel-alpha", String(panel.opacity / 100));
  const applyCompact = () => {
    compact.setAttribute("aria-pressed", String(panel.compact));
    const label = panel.compact ? "Show the controls" : "Hide the controls";
    compact.setAttribute("aria-label", label);
    compact.title = label;
  };
  opacity.value = String(panel.opacity);
  applyOpacity();
  applyCompact();
  opacity.addEventListener("input", () => {
    panel.opacity = Number(opacity.value) || 100;
    applyOpacity();
    savePanel();
  });
  compact.addEventListener("click", () => {
    panel.compact = !panel.compact;
    applyCompact();
    savePanel();
    deliver("compact", panel.compact);
  });

  listen("plugin-event", (event) => {
    const payload = event.payload || {};
    if (typeof payload.event !== "string") return;
    // `listen` hears the event whichever window it was sent to, so each plugin
    // window heard every plugin's, and another plugin's decoder of the same
    // name passed the permission check: with a copy of the Pet Soul Tracker
    // open beside it, every soul counted twice in both.
    if (payload.plugin !== undefined && payload.plugin !== id) return;
    // Sent to this plugin's widgets, not to its window.
    if (payload.event === "widget" || payload.event === "locked") return;
    if ((payload.event === "widget-message" || payload.event === "widget-moved") && !allowed("widgets")) return;
    if (typeof payload.seq !== "number") deliver(payload.event, payload.data);
    else if (early) early.push(payload);
    else deliverLogged(payload);
  });

  invoke("plugin_hello").then((hello) => {
    manifest = hello.manifest;
    document.title = manifest.name;
    document.querySelector(".pluginName").textContent = `${manifest.name} ${manifest.version}`;
    deliver("language", hello.language || "en");
    deliver("compact", panel.compact);
    if (hello.character) deliver("character", hello.character);
    if (typeof hello.game === "boolean") deliver("game", { running: hello.game });
    run = hello.run;
    try {
      const cursor = JSON.parse(localStorage.getItem(CURSOR_KEY) || "null");
      if (cursor?.run === run) delivered = Number(cursor.seq) || 0;
    } catch {}
    const logged = [...(hello.replay || []), ...early].sort((a, b) => a.seq - b.seq);
    early = null;
    for (const e of logged) deliverLogged(e);
    frame = document.createElement("iframe");
    frame.setAttribute("sandbox", "allow-scripts");
    frame.setAttribute("referrerpolicy", "no-referrer");
    // The scheme's base (http://a2plugin.localhost/ on Windows), then a plain
    // path: convertFileSrc would encode its "/", and the page's relative
    // links (style.css, plugin.js) would then resolve outside its folder.
    const base = convertFileSrc("", "a2plugin").replace(/\/*$/, "/");
    frame.src = `${base}${encodeURIComponent(id)}/index.html`;
    stage.appendChild(frame);
  }).catch((e) => {
    stage.textContent = `This plugin could not start: ${e}`;
  });
})();
