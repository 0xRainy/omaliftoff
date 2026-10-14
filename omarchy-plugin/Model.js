.pragma library
// Pure helpers for the OmaLiftoff widget. No QML types here so it can be unit
// tested with node (see tests/model.test.js).

var FINISHED = { 3: true, 4: true, 7: true, 9: true };
var PRECISE = { "SEC": true, "MIN": true, "HR": true, "": true };

// Upper bound for the files the widget reads (cache, nudge, test marker). The daemon's cache is
// ~150 KB for 40 launches; anything far larger is not ours and is not parsed.
var MAX_FILE_BYTES = 2 * 1024 * 1024;
function tooBig(n) { return !(n >= 0 && n <= MAX_FILE_BYTES); }

function oversizedCache() { return { launches: [], starred: [], error: "cache file too large (over 2 MiB); not read" }; }

function parseCache(text) {
  if (text && tooBig(text.length)) return oversizedCache();
  try {
    var c = JSON.parse(text || "{}");
    if (!c || !Array.isArray(c.launches)) return { launches: [], starred: [] };
    return c;
  } catch (e) {
    return { launches: [], starred: [], error: "unreadable cache" };
  }
}

function splitList(s) {
  return String(s || "").split(",").map(function (x) { return x.trim().toLowerCase(); })
    .filter(function (x) { return x.length > 0; });
}

function matchesProvider(l, list) {
  if (!list.length) return true;
  var n = String(l.provider || "").toLowerCase(), a = String(l.provider_abbrev || "").toLowerCase();
  return list.indexOf(n) !== -1 || (a && list.indexOf(a) !== -1);
}

// THE "is this launch overdue?" rule (mirrors Launch::current_until / overdue in model.rs; see
// the LL2 evidence there). Overdue never hides a launch: LL2's upcoming list is the ground truth. Exact/hour NETs: NET + 30 min. Day and coarser NETs are calendar labels
// (00:00 UTC of a day in the period): current through the end of the labelled UTC period, with
// quarter/half taken from the code. Unverified encodings: WK +7 days, FY end of calendar year,
// DEC end of decade, unknown end of day. Cases shared with Rust: tests/fixtures/overdue_cases.json.
function currentUntilMs(l) {
  var t = Date.parse(l.net), p = l.net_precision || "";
  if (isNaN(t)) return NaN;
  var d = new Date(t), y = d.getUTCFullYear(), m = d.getUTCMonth(); // m 0-based
  var dayEnd = Date.UTC(y, m, d.getUTCDate() + 1);
  var after = function (yy, m1) { return Date.UTC(yy, m1, 1); }; // first instant after 1-based month m1
  var end;
  switch (precisionKind(p)) {
    case "exact": case "hour": return t + 30 * 60 * 1000;
    case "week": end = dayEnd + 6 * 86400000; break;
    case "month": end = after(y, m + 1); break;
    case "quarter": end = after(y, (p.length === 2 ? Number(p[1]) : Math.floor(m / 3) + 1) * 3); break;
    case "half": end = after(y, Number(p[1]) * 6); break;
    case "year": case "fy": end = Date.UTC(y + 1, 0, 1); break;
    case "decade": end = Date.UTC(Math.floor(y / 10) * 10 + 10, 0, 1); break;
    default: end = dayEnd; // day, AM/PM, unknown
  }
  return Math.max(end, dayEnd);
}

function overdue(l, nowMs) {
  return l.status_id !== 6 && nowMs >= currentUntilMs(l);
}

// Launches worth counting down to, soonest first.
function upcoming(cache, nowMs, opts) {
  opts = opts || {};
  var filter = splitList(opts.providerFilter);
  var starred = (cache.starred || []).map(function (s) { return String(s).toLowerCase(); });
  return (cache.launches || []).filter(function (l) {
    if (FINISHED[l.status_id]) return false;
    var t = Date.parse(l.net);
    if (isNaN(t)) return false;
    if (!matchesProvider(l, filter)) return false;
    if (opts.starredOnly && !matchesProvider(l, starred)) return false;
    return true;
  }).sort(function (a, b) { return Date.parse(a.net) - Date.parse(b.net); });
}

// ---- TUI test flight (display only). The TUI writes testflight.json next to the
// cache with a heartbeat while its hidden test flight runs; it is ignored once
// stale, so a crashed TUI can't leave the bar stuck live.
var TEST_ID = "liftoff-test-flight";
var TEST_STALE_MS = 5000;

function parseTest(text) {
  if (text && tooBig(text.length)) return null;
  try { var t = JSON.parse(text || "{}"); return t && t.test && t.launch ? t : null; } catch (e) { return null; }
}

function testActive(marker, nowMs) {
  if (!marker || !marker.launch || !marker.heartbeat_ms) return false;
  var age = nowMs - marker.heartbeat_ms;
  return age >= -60000 && age <= (marker.stale_ms || TEST_STALE_MS);
}

function isTest(l) { return !!l && l.id === TEST_ID; }

// Cache with the live test flight first (copy; the real cache is never modified).
function withTest(cache, marker, nowMs) {
  if (!testActive(marker, nowMs)) return cache;
  var out = Object.assign({}, cache);
  out.launches = [marker.launch].concat((cache.launches || []).filter(function (l) { return l.id !== TEST_ID; }));
  out.flight_plans = Object.assign({}, cache.flight_plans || {});
  if (marker.flight_plan) out.flight_plans[TEST_ID] = marker.flight_plan;
  out.test = marker;
  return out;
}

// Upcoming, but an active test flight always comes first.
function upcomingWithTest(cache, nowMs, opts) {
  var u = upcoming(cache, nowMs, opts);
  if (!cache || !cache.test) return u;
  var t = (cache.launches || []).filter(isTest)[0];
  return t ? [t].concat(u.filter(function (l) { return !isTest(l); })) : u;
}

function pad2(n) { return (n < 10 ? "0" : "") + n; }

// Whole seconds since T-0, floored (negative before launch). Same rule as the TUI
// (schedule.rs rel_secs), so bar and TUI show the same second at the same instant.
function relSecs(netIso, nowMs) {
  return Math.floor((nowMs - Date.parse(netIso)) / 1000);
}

// Delay until just after the next whole second of the skew-corrected clock
// (local + cache.sync.clock_offset_ms, as the TUI uses), so 1 Hz ticks flip with it.
function msToNextSecond(nowMs, cache) {
  var off = cache && cache.sync && cache.sync.clock_offset_ms ? cache.sync.clock_offset_ms : 0;
  var t = nowMs + off;
  return 1000 - (((t % 1000) + 1000) % 1000) + 5;
}

function countdown(netIso, nowMs) {
  var s = -relSecs(netIso, nowMs);
  var sign = s >= 0 ? "-" : "+";
  s = Math.abs(s);
  var d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  if (d > 0) return "T" + sign + d + "d " + pad2(h) + "h";
  return "T" + sign + pad2(h) + ":" + pad2(m) + ":" + pad2(sec);
}

var DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
var MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

// LL2 net_precision.abbrev (2.3.0 /config/net_precisions/): SEC MIN HR AM PM DAY WK M Q1-Q4 H1 H2 Y FY DEC.
// Legacy/never-sent codes (HALF, TOD, W, Q) are mapped harmlessly. Mirrors Precision in model.rs.
function precisionKind(p) {
  p = p || "";
  if (p === "SEC" || p === "MIN" || p === "") return "exact";
  if (p === "HR") return "hour";
  if (p === "AM" || p === "PM" || p === "DAY" || p === "HALF" || p === "TOD") return "day";
  if (p === "WK" || p === "W") return "week";
  if (p === "M") return "month";
  if (/^Q[1-4]?$/.test(p)) return "quarter";
  if (p === "H1" || p === "H2") return "half";
  if (p === "Y") return "year";
  if (p === "FY") return "fy";
  if (p === "DEC") return "decade";
  return "unknown";
}

// THE only NET formatter (mirrors Launch::net_text in model.rs; see its evidence notes).
// fmt: "short" | "long" | "clock" | "column". Local time only for exact/hour; day and coarser
// are calendar labels -> UTC fields. AM/PM/WK/FY/DEC encodings are unverified -> "~".
function netText(l, fmt) {
  fmt = fmt || "short";
  var d = new Date(Date.parse(l.net));
  var p = l.net_precision || "";
  var md = MONTHS[d.getUTCMonth()] + " " + d.getUTCDate();
  var y = d.getUTCFullYear();
  var hm = pad2(d.getHours()) + ":" + pad2(d.getMinutes());
  var lmd = MONTHS[d.getMonth()] + " " + d.getDate();
  switch (precisionKind(p)) {
    case "exact":
      return fmt === "clock" ? hm : fmt === "long" ? DAYS[d.getDay()] + " " + lmd + " " + hm : lmd + " " + hm;
    case "hour":
      return fmt === "clock" ? "~" + hm + " ±1h" : fmt === "long" ? DAYS[d.getDay()] + " " + lmd + " ~" + hm + " ±1h" : lmd + " ~" + pad2(d.getHours()) + ":00 (±1h)";
    case "day":
      if (p === "AM" || p === "PM") return fmt === "column" ? "~" + md : "~" + md + (p === "AM" ? " (morning, site time)" : " (afternoon, site time)");
      return fmt === "long" ? DAYS[d.getUTCDay()] + " " + md : md;
    case "week": return fmt === "column" ? "~" + md : "~" + md + " (week)";
    case "month": return MONTHS[d.getUTCMonth()] + " " + y;
    case "quarter": return (p.length === 2 ? p : "Q" + (Math.floor(d.getUTCMonth() / 3) + 1)) + " " + y;
    case "half": return p + " " + y;
    case "year": return String(y);
    case "fy": return fmt === "column" ? "~" + y : "~" + y + " (fiscal year)";
    case "decade": return "~" + Math.floor(y / 10) * 10 + "s";
    default: return "~" + md;
  }
}

// Don't fake precision: only exact/hour NETs tick; coarse NETs always show their label.
function displayCountdown(l, nowMs) {
  if (overdue(l, nowMs)) return "NET " + netText(l, "long") + " · awaiting update";
  if (PRECISE[l.net_precision || ""]) return countdown(l.net, nowMs);
  return "NET " + netText(l, "long");
}

function shortRocket(l) {
  var r = String(l.rocket || "").trim();
  return r || String(l.name || "").split("|")[0].trim();
}

// Nerd Font nf-md-rocket_launch (U+F14DE), painted as its own Text so it can be colored.
var ICON = String.fromCodePoint(0xF14DE);

// Bar label WITHOUT the icon. stage: optional current flight event ("MECO").
function barText(l, nowMs, showVehicle, stage) {
  if (!l) return "—";
  if (isTest(l)) return "Test Flight · " + (stage ? stage + " · " : "") + displayCountdown(l, nowMs);
  return (showVehicle ? shortRocket(l) + " · " : "") + (stage ? stage + " · " : "") + displayCountdown(l, nowMs);
}

// Icon state: "bad" (failure / partial failure: red, static) > "live" (accent, pulsing)
// > "bad" (hold: red) > "normal" (plain foreground).
function iconState(l, nowMs, stage) {
  if (!l) return "normal";
  var cls = l.status_class || "";
  if (cls === "failure" || l.status_id === 4 || l.status_id === 7) return "bad";
  var t = Date.parse(l.net);
  var live = !!stage || l.status_id === 6 || !!l.webcast_live ||
    (!!PRECISE[l.net_precision || ""] && t <= nowMs && t > nowMs - 3 * 3600000 && cls !== "success");
  if (live) return "live";
  if (cls === "hold" || l.status_id === 5) return "bad";
  return "normal";
}

// Text next to the icon? textMode: "auto" | "always" | "never".
// auto: text only in the center section of a horizontal bar; icon-only in left/right/tray
// and on vertical bars. Unknown region (shell didn't expose one) -> show text.
function showText(textMode, region, vertical) {
  var m = String(textMode || "auto").toLowerCase();
  if (m === "always") return true;
  if (m === "never") return false;
  if (vertical) return false;
  var r = String(region || "");
  return r === "" || r === "center";
}

function isLive(l, nowMs, windowMinutes) {
  if (!l) return false;
  if (l.status_id === 6 || l.webcast_live) return true;
  if (!PRECISE[l.net_precision || ""]) return false; // a calendar label has no T-0
  var dt = Date.parse(l.net) - nowMs;
  return dt <= (windowMinutes || 60) * 60000 && dt > -30 * 60000;
}

// Returns a palette key; the widget maps it to theme or traffic colors.
function colorKey(l) {
  return l ? (l.status_class || "tbd") : "tbd";
}

function fmtRel(secs) {
  var a = Math.abs(secs), sign = secs < 0 ? "-" : "+";
  if (a >= 3600) return "T" + sign + pad2(Math.floor(a / 3600)) + ":" + pad2(Math.floor((a % 3600) / 60)) + ":" + pad2(a % 60);
  return "T" + sign + pad2(Math.floor(a / 60)) + ":" + pad2(a % 60);
}

// Next countdown/flight milestone: SpaceX flight plan (cache.flight_plans) first,
// LL2 timeline otherwise. Returns { label, description, at, in, text } or null.
function nextMilestone(cache, l, nowMs) {
  if (!l) return null;
  var plan = cache && cache.flight_plans ? cache.flight_plans[l.id] : null;
  var ev = plan && plan.events && plan.events.length ? plan.events : (l.milestones || []);
  var tRel = relSecs(l.net, nowMs);
  for (var i = 0; i < ev.length; i++) {
    if (ev[i].offset_secs > tRel) {
      var e = ev[i];
      return { label: e.label, description: e.description, at: fmtRel(e.offset_secs), in: e.offset_secs - tRel,
        text: e.label + " at " + fmtRel(e.offset_secs) };
    }
  }
  return null;
}

function parseNudge(text) {
  if (text && tooBig(text.length)) return {};
  try { var n = JSON.parse(text || "{}"); return n && typeof n === "object" ? n : {}; } catch (e) { return {}; }
}

// Same rule as the Rust side (nudge.rs): an absolute T-0 for one launch, ignored
// if LL2's NET has since moved more than 120 s (hold / new attempt). Clock skew:
// shifting every NET by -offset is equivalent to using a corrected "now".
function applyClock(cache, nudge) {
  if (!cache || !Array.isArray(cache.launches)) return cache;
  var off = cache.sync && cache.sync.clock_offset_ms ? cache.sync.clock_offset_ms : 0;
  var hasNudge = nudge && nudge.launch_id && nudge.t0 && nudge.base_net;
  if (!off && !hasNudge) return cache;
  var out = Object.assign({}, cache);
  out.launches = cache.launches.map(function (l) {
    var net = Date.parse(l.net);
    if (hasNudge && l.id === nudge.launch_id && Math.abs(net - Date.parse(nudge.base_net)) <= 120000) net = Date.parse(nudge.t0);
    if (off) net -= off;
    if (net === Date.parse(l.net)) return l;
    return Object.assign({}, l, { net: new Date(net).toISOString() });
  });
  return out;
}

function eventsFor(cache, l) {
  if (!l) return [];
  var plan = cache && cache.flight_plans ? cache.flight_plans[l.id] : null;
  return plan && plan.events && plan.events.length ? plan.events : (l.milestones || []);
}

// Live stage info from flight plan events + the clock. Returns null unless the
// launch is in flight or within T-10 min .. last event + 1 min and events exist.
// { current: {label, at}|null, next: {label, at, in}|null, tRel, source }
function liveStage(cache, l, nowMs) {
  var ev = eventsFor(cache, l);
  if (!l || !ev.length) return null;
  var tRel = relSecs(l.net, nowMs);
  var last = ev[ev.length - 1].offset_secs;
  var inFlight = l.status_id === 6;
  if (!inFlight && !(PRECISE[l.net_precision || ""] && tRel >= -600 && tRel <= last + 60)) return null;
  var cur = null, nxt = null;
  for (var i = 0; i < ev.length; i++) {
    if (ev[i].offset_secs <= tRel) cur = ev[i];
    else { nxt = ev[i]; break; }
  }
  var plan = cache && cache.flight_plans ? cache.flight_plans[l.id] : null;
  return {
    tRel: tRel,
    source: plan && plan.events && plan.events.length ? "spacex.com" : "LL2",
    current: cur ? { label: cur.label, description: cur.description || "", at: fmtRel(cur.offset_secs) } : null,
    next: nxt ? { label: nxt.label, description: nxt.description || "", at: fmtRel(nxt.offset_secs), in: nxt.offset_secs - tRel } : null
  };
}

function shortSecs(s) {
  s = Math.max(0, Math.round(s));
  if (s >= 3600) return Math.floor(s / 3600) + "h" + pad2(Math.floor((s % 3600) / 60)) + "m";
  if (s >= 60) return Math.floor(s / 60) + "m" + pad2(s % 60) + "s";
  return s + "s";
}

function ago(iso, nowMs) {
  if (!iso) return "never";
  var m = Math.floor((nowMs - Date.parse(iso)) / 60000);
  if (m < 1) return "just now";
  if (m < 60) return m + "m ago";
  return Math.floor(m / 60) + "h " + (m % 60) + "m ago";
}

function hidden(l, nowMs, hideBeyondDays) {
  if (!l) return false;
  if (!hideBeyondDays || hideBeyondDays <= 0) return false;
  return Date.parse(l.net) - nowMs > hideBeyondDays * 86400000;
}

// "$HOME/x", "${HOME}/x" and "~/x" -> absolute path, so the binary can be exec'd without a shell.
function expandHome(p, home) {
  p = String(p || "");
  var m = /^(?:\$HOME|\$\{HOME\}|~)(\/.*)?$/.exec(p);
  return m ? String(home || "") + (m[1] || "") : p;
}

// Node test hook (ignored by QML).
if (typeof module !== "undefined") module.exports = {
  MAX_FILE_BYTES: MAX_FILE_BYTES, tooBig: tooBig, oversizedCache: oversizedCache, expandHome: expandHome, parseCache: parseCache, parseTest: parseTest, testActive: testActive, withTest: withTest, upcomingWithTest: upcomingWithTest, isTest: isTest, TEST_ID: TEST_ID, upcoming: upcoming, currentUntilMs: currentUntilMs, overdue: overdue, countdown: countdown, relSecs: relSecs, msToNextSecond: msToNextSecond, displayCountdown: displayCountdown, netText: netText, precisionKind: precisionKind,
  barText: barText, iconState: iconState, showText: showText, ICON: ICON, nextMilestone: nextMilestone, liveStage: liveStage, applyClock: applyClock, parseNudge: parseNudge, shortSecs: shortSecs, fmtRel: fmtRel, isLive: isLive, colorKey: colorKey, hidden: hidden, ago: ago, shortRocket: shortRocket
};
