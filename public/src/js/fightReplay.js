// Fight replay: a saved fight in Details, played back from its start, or
// scrubbed to any moment, at a chosen speed. Every figure the window shows
// for the fight (the party's damage, DPS and share, the stats, healing, the
// skills' damage, hits and crits) is then what had happened by that moment;
// the fight timeline (chart, buffs, casts) is drawn once for the whole fight
// and shown up to the moment, the rest faint.
//
// What the record has to cut by: each skill row's hit times (ms into the
// fight; heals too, from meter 2.0.56), and each death's time. It keeps no
// per-hit amounts or flags, so a skill's damage and its crit, back, parry...
// counts are given out evenly over its hits up to the moment, as the DPS
// chart already does with its damage. Hit counts and deaths are exact. Not
// timed at all, and so left at the whole fight's (and greyed): Damage Taken,
// the skills' Min / Max, and the buffs' uptimes.
//
// Saved fights only: a live fight's details arrive merged per target from
// the backend every 2 s, and replaying that would mean holding a second copy
// of the fight in step with it.
//
// details.js hands this file its hooks (`window.fightReplay.attach`), and
// draws the figures from the cut ones with `renderFigures`.

// ── The cut (no DOM; Node can load this part to test it) ──

const REPLAY_COUNT_FIELDS = [
  "crit", "parry", "back", "frontal", "perfect", "double", "shieldBlock", "ironWall",
  "regeneration", "perfectBlock", "miss", "resist", "regen",
  "multiHitCount", "multiHitDamage", "multiHitHits",
];

// Hits at or before `t` in ascending `ts`.
const replayHitsUpTo = (ts, t) => {
  let lo = 0;
  let hi = ts.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (ts[mid] <= t) lo = mid + 1;
    else hi = mid;
  }
  return lo;
};

const replayIndexRows = (rows) => (Array.isArray(rows) ? rows : [])
  .filter((row) => row && typeof row === "object")
  .map((row) => {
    const ts = (Array.isArray(row.hitTimestamps) ? row.hitTimestamps : [])
      .map(Number).filter(Number.isFinite).sort((a, b) => a - b);
    return { row, ts };
  });

// Built once when a replay starts: each row's hit times sorted, and the
// whole fight's figures the cut ones are scaled against.
const buildReplayModel = (record) => {
  const details = record?.details && typeof record.details === "object" ? record.details : {};
  const damage = replayIndexRows(details.skills);
  const heals = replayIndexRows(details.healSkills);
  const lastHit = [...damage, ...heals].reduce((m, { ts }) => (ts.length ? Math.max(m, ts[ts.length - 1]) : m), 0);
  const durationMs = Number(details.battleTime) || Number(record?.durationMs) || lastHit;
  const fullSkillDamage = damage.reduce((sum, { row }) => sum + (Number(row.dmg) || 0), 0);
  return {
    record,
    details,
    damage,
    heals,
    durationMs,
    fullSkillDamage,
    totalDamage: Number(record?.totalDamage) || fullSkillDamage,
    damageTimed: damage.some(({ ts }) => ts.length > 0),
    // A fight saved before heals had times: its healing stays the whole
    // fight's (with a note) rather than all arriving at the end.
    healTimed: heals.some(({ row, ts }) => ts.length > 0 && (Number(row.dmg) || 0) > 0),
  };
};

// A row as it stood at `t`: null before its first hit, the row itself once
// all of it has happened. A row with no hit times (an older or partial
// record) counts at the end, unless `untimedWhole`.
const replayCutRow = ({ row, ts }, t, atEnd, untimedWhole) => {
  const n = ts.length;
  if (!n) return untimedWhole || atEnd ? row : null;
  const k = atEnd ? n : replayHitsUpTo(ts, t);
  if (k >= n) return row;
  if (k <= 0) return null;
  const f = k / n;
  const out = { ...row, hitTimestamps: ts.slice(0, k) };
  const time = Number(row.time) || 0;
  out.time = time === n ? k : Math.round(time * f);
  out.dmg = Math.round((Number(row.dmg) || 0) * f);
  for (const field of REPLAY_COUNT_FIELDS) {
    if (row[field] !== undefined) out[field] = Math.round((Number(row[field]) || 0) * f);
  }
  // Saved before the flags were renamed.
  if (row.smite !== undefined && row.regeneration === undefined) out.regeneration = Math.round((Number(row.smite) || 0) * f);
  if (row.powershard !== undefined && row.perfectBlock === undefined) out.perfectBlock = Math.round((Number(row.powershard) || 0) * f);
  return out;
};

// The record's details as they stood `t` ms into the fight, in the shape
// load_fight returns them, plus the target's per-actor damage and total.
const cutReplayAt = (model, tMs) => {
  const t = Math.max(0, Number(tMs) || 0);
  const atEnd = t >= model.durationMs;
  const skills = [];
  const actorDamage = {};
  let skillDamage = 0;
  for (const entry of model.damage) {
    const row = replayCutRow(entry, t, atEnd, false);
    if (!row) continue;
    skills.push(row);
    const dmg = Number(row.dmg) || 0;
    skillDamage += dmg;
    const id = Number(row.actorId);
    if (id > 0) actorDamage[id] = (actorDamage[id] || 0) + dmg;
  }
  const healSkills = [];
  for (const entry of model.heals) {
    const row = replayCutRow(entry, t, atEnd, !model.healTimed);
    if (row) healSkills.push(row);
  }
  const deathsIn = Array.isArray(model.details.deaths) ? model.details.deaths : null;
  const deaths = deathsIn && (atEnd ? deathsIn : deathsIn
    .filter((d) => Number(d?.tsMs) <= t)
    .map((d) => (d.revivedMs != null && Number(d.revivedMs) > t ? { actorId: d.actorId, tsMs: d.tsMs } : d)));
  // Damage the record names no actor for keeps the whole fight's share.
  const totalDamage = atEnd
    ? model.totalDamage
    : model.fullSkillDamage > 0 ? Math.round(model.totalDamage * (skillDamage / model.fullSkillDamage)) : skillDamage;
  // DPS is over the time so far, at least a second (as Details' Avg DPS is).
  const battleTime = atEnd ? model.durationMs : Math.max(1000, Math.round(t));
  const details = { ...model.details, battleTime, totalTargetDamage: totalDamage, skills, healSkills };
  if (deathsIn) details.deaths = deaths;
  return { t: atEnd ? model.durationMs : t, atEnd, details, actorDamage, totalDamage, battleTime };
};

if (typeof module !== "undefined" && module.exports) {
  module.exports = { buildReplayModel, cutReplayAt, replayHitsUpTo };
}

// ── The controls ──

(() => {
  if (typeof window === "undefined" || typeof document === "undefined") return;

  const SPEEDS = [0.5, 1, 2, 4, 8, 16, 32];
  const SPEED_KEY = "a2.details.replaySpeed";
  // Figures are drawn at most this often while playing (the scrubber, the
  // time and the timeline's mask move every frame); less often in a fight so
  // big that drawing them takes long (Details' peak DPS goes through every
  // hit), so they never take more than about a sixth of the time.
  const FIGURE_INTERVAL_MS = 100;
  const FIGURE_SHARE = 6;
  // How faint the timeline is past the moment.
  const FUTURE_ALPHA = 0.16;

  const t = (key, fallback) => window.i18n?.t?.(key, fallback) ?? fallback;
  const clock = (ms) => {
    const whole = Math.max(0, Math.floor((Number(ms) || 0) / 1000));
    return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}`;
  };

  const attach = (hooks) => {
    if (!hooks?.root || !hooks?.timeline) return;
    const panel = hooks.root;
    const timeline = hooks.timeline;
    const scroll = hooks.timelineScroll;

    let speed = 4;
    try {
      const saved = Number(localStorage.getItem(SPEED_KEY));
      if (SPEEDS.includes(saved)) speed = saved;
    } catch { /* default */ }

    // ── DOM ──
    const bar = document.createElement("div");
    bar.className = "fightReplayBar";
    bar.hidden = true;
    bar.setAttribute("role", "group");
    bar.dataset.i18nAriaLabel = "details.replay.label";
    bar.setAttribute("aria-label", t("details.replay.label", "Replay"));

    const controls = document.createElement("div");
    controls.className = "fightReplayControls";

    const playBtn = document.createElement("button");
    playBtn.type = "button";
    playBtn.className = "fightReplayBtn fightReplayPlay";
    playBtn.innerHTML = '<i data-lucide="play" class="fightReplayIconPlay" aria-hidden="true"></i>'
      + '<i data-lucide="pause" class="fightReplayIconPause" aria-hidden="true"></i>';

    const label = document.createElement("span");
    label.className = "fightReplayLabel";
    label.dataset.i18n = "details.replay.title";
    label.textContent = t("details.replay.title", "Replay");

    const timeEl = document.createElement("span");
    timeEl.className = "fightReplayTime";
    timeEl.setAttribute("aria-hidden", "true");

    const speedSel = document.createElement("select");
    speedSel.className = "fightReplaySpeed";
    speedSel.dataset.i18nTitle = "details.replay.speed";
    speedSel.dataset.i18nAriaLabel = "details.replay.speed";
    speedSel.title = t("details.replay.speed", "Playback speed");
    speedSel.setAttribute("aria-label", speedSel.title);
    for (const s of SPEEDS) {
      const opt = document.createElement("option");
      opt.value = String(s);
      opt.textContent = `${s}x`;
      speedSel.appendChild(opt);
    }
    speedSel.value = String(speed);

    controls.append(playBtn, label, timeEl, speedSel);

    const scrubWrap = document.createElement("div");
    scrubWrap.className = "fightReplayScrubWrap";
    const scrub = document.createElement("input");
    scrub.type = "range";
    scrub.className = "fightReplayScrub";
    scrub.min = "0";
    scrub.max = "0";
    scrub.step = "any";
    scrub.value = "0";
    scrub.dataset.i18nAriaLabel = "details.replay.position";
    scrub.setAttribute("aria-label", t("details.replay.position", "Replay position"));
    scrubWrap.appendChild(scrub);

    const exitBtn = document.createElement("button");
    exitBtn.type = "button";
    exitBtn.className = "fightReplayBtn fightReplayExit";
    exitBtn.innerHTML = '<i data-lucide="x" aria-hidden="true"></i>';
    exitBtn.dataset.i18nTitle = "details.replay.exit";
    exitBtn.dataset.i18nAriaLabel = "details.replay.exit";
    exitBtn.title = t("details.replay.exit", "End replay (show the whole fight)");
    exitBtn.setAttribute("aria-label", exitBtn.title);

    const note = document.createElement("div");
    note.className = "fightReplayNote";
    note.dataset.i18n = "details.replay.wholeFightNote";
    note.textContent = t("details.replay.wholeFightNote",
      "Damage Taken, Min / Max and buff uptimes are for the whole fight; damage and crits are shared evenly over each skill's hits.");
    const healNote = document.createElement("div");
    healNote.className = "fightReplayNote fightReplayHealNote";
    healNote.dataset.i18n = "details.replay.noHealTiming";
    healNote.textContent = t("details.replay.noHealTiming",
      "Healing in this fight has no times (an older version saved it): it shows the whole fight.");

    bar.append(controls, scrubWrap, exitBtn, note, healNote);

    const axis = timeline.querySelector(".fightTimelineAxis");
    if (axis?.nextSibling) timeline.insertBefore(bar, axis.nextSibling);
    else timeline.appendChild(bar);

    const head = document.createElement("div");
    head.className = "fightReplayHead";
    head.hidden = true;
    head.setAttribute("aria-hidden", "true");
    timeline.appendChild(head);

    try { window.lucide?.createIcons?.(); } catch { /* text-less buttons keep their labels */ }

    // ── State ──
    let record = null;
    let model = null;
    let active = false;
    let playing = false;
    let at = 0; // ms into the fight
    let frame = 0;
    let lastFrameTs = 0;
    let lastFiguresTs = 0;
    let figuresPending = false;
    let cutSeq = 0;
    let geometry = null; // the timeline's lane edges, re-read on resize
    let ownRender = false;
    let figureCostMs = 0;

    const durationMs = () => Number(hooks.durationMs?.()) || model?.durationMs || 0;

    const syncPlayButton = () => {
      bar.classList.toggle("isPlaying", playing);
      const key = playing ? "details.replay.pause" : "details.replay.play";
      const text = playing ? t(key, "Pause") : t(key, "Play replay");
      playBtn.dataset.i18nTitle = key;
      playBtn.dataset.i18nAriaLabel = key;
      playBtn.title = text;
      playBtn.setAttribute("aria-label", text);
    };

    const readGeometry = () => {
      if (!scroll) return null;
      const m = hooks.metrics?.() || { gutter: 0, right: 0 };
      const left = m.gutter;
      const right = scroll.clientWidth - m.right;
      if (!(right > left)) return null;
      return { left, right, top: scroll.offsetTop, height: scroll.clientHeight, x0: scroll.offsetLeft };
    };

    // The timeline up to the moment, the rest faint, and a line at it.
    const paintTimeline = () => {
      if (!scroll) return;
      if (!active) {
        scroll.style.removeProperty("mask-image");
        scroll.style.removeProperty("-webkit-mask-image");
        head.hidden = true;
        return;
      }
      if (!geometry) geometry = readGeometry();
      const dur = durationMs();
      if (!geometry || dur <= 0) return;
      const x = geometry.left + Math.min(1, at / dur) * (geometry.right - geometry.left);
      const faint = `rgba(0,0,0,${FUTURE_ALPHA})`;
      const mask = `linear-gradient(to right, #000 ${x.toFixed(1)}px, ${faint} ${x.toFixed(1)}px, `
        + `${faint} ${geometry.right}px, #000 ${geometry.right}px)`;
      scroll.style.setProperty("-webkit-mask-image", mask);
      scroll.style.setProperty("mask-image", mask);
      head.hidden = false;
      head.style.transform = `translateX(${(geometry.x0 + x).toFixed(1)}px)`;
      head.style.top = `${geometry.top}px`;
      head.style.height = `${geometry.height}px`;
    };

    const paintControls = () => {
      const dur = durationMs();
      scrub.max = String(dur);
      scrub.value = String(active ? at : dur);
      const shown = active ? at : dur;
      timeEl.textContent = `${clock(shown)} / ${clock(dur)}`;
      scrub.setAttribute("aria-valuetext", t("details.replay.positionText", "{at} of {total}")
        .replace("{at}", clock(shown)).replace("{total}", clock(dur)));
      scrub.style.setProperty("--range-pct", `${dur > 0 ? ((shown / dur) * 100).toFixed(2) : 100}%`);
    };

    // Cut the fight at the moment and draw the figures from it. Resolves in
    // microtasks (load_fight's details need no IPC), so a full redraw the
    // observer caught is replaced before it is painted.
    const drawFigures = () => {
      if (!active || !model) return;
      const seq = ++cutSeq;
      // HEAL in a fight whose heals have no times: every figure there is
      // healing, so all of it stays the whole fight's (the note says so).
      if (!model.healTimed && panel.classList.contains("isHealMode")) {
        ownRender = true;
        try { hooks.renderFigures(null); } finally { observer.takeRecords(); ownRender = false; }
        return;
      }
      const cut = cutReplayAt(model, at);
      const sel = hooks.selection?.() || {};
      const attackerIds = Array.isArray(sel.attackerIds) && sel.attackerIds.length ? sel.attackerIds : null;
      const row = { id: null, job: "", name: record?.bossName || "" };
      const load = (ids) => {
        // getDetails reads the override before its first await.
        const prev = window._historyDetailsOverride;
        window._historyDetailsOverride = cut.details;
        try {
          return hooks.getDetails(row, {
            targetId: record?.targetId,
            attackerIds: ids,
            totalTargetDamage: cut.totalDamage,
            showSkillIcons: !ids,
          });
        } finally {
          window._historyDetailsOverride = prev;
        }
      };
      Promise.all([load(attackerIds), attackerIds ? load(null) : null])
        .then(([details, unfiltered]) => {
          if (seq !== cutSeq || !active || !details) return;
          const base = hooks.targets?.()?.[0] || { targetId: record?.targetId };
          const target = { ...base, actorDamage: cut.actorDamage, totalDamage: cut.totalDamage, battleTime: cut.battleTime };
          ownRender = true;
          const started = performance.now();
          try {
            hooks.renderFigures({ details, unfiltered: unfiltered || details, target });
          } finally {
            observer.takeRecords();
            ownRender = false;
          }
          const took = performance.now() - started;
          figureCostMs = figureCostMs ? figureCostMs * 0.7 + took * 0.3 : took;
        })
        .catch((e) => window.javaBridge?.logToDebug?.(`Replay figures failed: ${e?.message || e}`));
    };

    const requestFigures = (now = performance.now(), force = false) => {
      const every = playing ? Math.max(FIGURE_INTERVAL_MS, figureCostMs * FIGURE_SHARE) : figureCostMs * 3;
      if (!force && now - lastFiguresTs < every) {
        figuresPending = true;
        return;
      }
      figuresPending = false;
      lastFiguresTs = now;
      drawFigures();
    };

    const tick = (now) => {
      frame = 0;
      if (!active) return;
      if (playing) {
        const dt = lastFrameTs ? Math.min(250, now - lastFrameTs) : 0;
        lastFrameTs = now;
        at += dt * speed;
        if (at >= durationMs()) {
          // The end: back to the whole fight.
          exit();
          return;
        }
        requestFigures(now);
      } else if (figuresPending) {
        requestFigures(now);
      }
      paintControls();
      paintTimeline();
      if (playing || figuresPending) frame = requestAnimationFrame(tick);
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(tick);
    };

    const enter = () => {
      if (active) return true;
      record = hooks.record?.() || null;
      if (!record) return false;
      if (!model || model.record !== record) model = buildReplayModel(record);
      if (!model.damageTimed || durationMs() <= 0) return false;
      active = true;
      at = 0;
      geometry = null;
      panel.classList.add("isReplaying");
      bar.classList.add("isActive");
      syncHealNote();
      return true;
    };

    const exit = () => {
      const was = active;
      active = false;
      playing = false;
      cutSeq++;
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      lastFrameTs = 0;
      figuresPending = false;
      panel.classList.remove("isReplaying");
      bar.classList.remove("isActive");
      syncPlayButton();
      paintTimeline();
      paintControls();
      if (was && hooks.isOpen?.()) {
        ownRender = true;
        try { hooks.renderFigures(null); } finally { observer.takeRecords(); ownRender = false; }
      }
    };

    const play = () => {
      if (!active) {
        if (!enter()) return;
      } else if (at >= durationMs()) {
        at = 0;
      }
      playing = true;
      lastFrameTs = 0;
      syncPlayButton();
      requestFigures(performance.now(), true);
      schedule();
    };
    const pause = () => {
      playing = false;
      syncPlayButton();
      requestFigures(performance.now(), true);
      schedule();
    };

    const seek = (ms) => {
      if (!active && !enter()) return;
      at = Math.max(0, Math.min(durationMs(), Number(ms) || 0));
      figuresPending = true;
      schedule();
    };

    // ── Input ──
    playBtn.addEventListener("click", () => (playing ? pause() : play()));
    exitBtn.addEventListener("click", () => {
      exit();
      playBtn.focus();
    });
    speedSel.addEventListener("change", () => {
      const next = Number(speedSel.value);
      if (!SPEEDS.includes(next)) return;
      speed = next;
      try { localStorage.setItem(SPEED_KEY, String(next)); } catch { /* not kept */ }
    });
    // Dragging holds playback and lets go where it was.
    let resumeAfterDrag = false;
    scrub.addEventListener("pointerdown", () => {
      resumeAfterDrag = playing;
      if (playing) pause();
    });
    const endDrag = () => {
      if (resumeAfterDrag) {
        resumeAfterDrag = false;
        play();
      }
    };
    scrub.addEventListener("pointerup", endDrag);
    scrub.addEventListener("pointercancel", endDrag);
    scrub.addEventListener("input", () => seek(scrub.value));
    // Arrows a second (Shift: ten), Page keys thirty, Home / End the ends;
    // Space plays or pauses.
    scrub.addEventListener("keydown", (event) => {
      const dur = durationMs();
      const step = { ArrowLeft: -1000, ArrowDown: -1000, ArrowRight: 1000, ArrowUp: 1000, PageDown: -30000, PageUp: 30000 }[event.key];
      if (step !== undefined) {
        event.preventDefault();
        seek((active ? at : dur) + step * (event.shiftKey && Math.abs(step) === 1000 ? 10 : 1));
      } else if (event.key === "Home") {
        event.preventDefault();
        seek(0);
      } else if (event.key === "End") {
        event.preventDefault();
        seek(dur);
      } else if (event.key === " " || event.key === "Spacebar") {
        event.preventDefault();
        if (playing) pause();
        else play();
      } else if (event.key === "Escape" && active) {
        event.preventDefault();
        event.stopPropagation();
        exit();
      }
    });
    bar.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && active && event.target !== scrub) {
        event.preventDefault();
        event.stopPropagation();
        exit();
        playBtn.focus();
      }
    });

    // ── Keeping in step with the window ──
    const syncHealNote = () => {
      bar.classList.toggle("noHealTiming", !!model && !model.healTimed && panel.classList.contains("isHealMode"));
    };

    // Shown for a saved fight that has hit times; a new fight or a closed
    // panel ends a replay.
    const syncAvailability = () => {
      const open = !!hooks.isOpen?.();
      const next = open ? hooks.record?.() || null : null;
      if (next !== record || !open) {
        if (active) exit();
        record = next;
        model = null;
      }
      const timed = !!record && (Array.isArray(record.details?.skills) ? record.details.skills : [])
        .some((s) => Array.isArray(s?.hitTimestamps) && s.hitTimestamps.length > 0);
      bar.hidden = !record;
      bar.classList.toggle("isUnavailable", !!record && !timed);
      playBtn.disabled = !!record && !timed;
      scrub.disabled = !!record && !timed;
      if (record && !timed) {
        playBtn.title = t("details.replay.unavailable", "This fight was saved without hit times, so it cannot be replayed");
      } else {
        syncPlayButton();
      }
      paintControls();
    };

    // A full redraw of the figures (a player picked, DMG / HEAL, a sort, the
    // peak switch) is drawn again from the cut while replaying.
    const observer = new MutationObserver(() => {
      if (ownRender) return;
      syncAvailability();
      syncHealNote();
      if (active) {
        figuresPending = false;
        lastFiguresTs = performance.now();
        drawFigures();
      }
    });
    const watched = hooks.figureEls?.() || [];
    for (const el of watched) if (el) observer.observe(el, { childList: true, subtree: true, characterData: true });
    new MutationObserver(() => {
      syncAvailability();
      syncHealNote();
    }).observe(panel, { attributes: true, attributeFilter: ["class"] });

    if (typeof ResizeObserver === "function" && scroll) {
      new ResizeObserver(() => {
        geometry = null;
        if (active) paintTimeline();
      }).observe(scroll);
    }
    window.i18n?.onChange?.(() => {
      syncPlayButton();
      syncAvailability();
    });

    syncPlayButton();
    syncAvailability();
  };

  window.fightReplay = { attach };
})();
