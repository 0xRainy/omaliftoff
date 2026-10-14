// node omarchy-plugin/tests/model.test.js  — tests the widget's pure logic.
const fs = require("fs"), path = require("path"), assert = require("assert");
const src = fs.readFileSync(path.join(__dirname, "..", "Model.js"), "utf8").replace(".pragma library", "");
const m = {}; new Function("module", src)(m); const M = m.exports;
const now = Date.parse("2026-10-13T18:00:00Z");
const L = (o) => Object.assign({ id: "x", name: "Falcon 9 Block 5 | Test", rocket: "Falcon 9", provider: "SpaceX", provider_abbrev: "SpX",
  status_id: 1, status_class: "go", net: "2026-10-13T20:14:07Z", net_precision: "MIN" }, o);
assert.strictEqual(M.countdown("2026-10-13T20:14:07Z", now), "T-02:14:07");
assert.strictEqual(M.countdown("2026-10-16T22:00:00Z", now), "T-3d 04h");
assert.strictEqual(M.countdown("2026-10-13T17:54:48Z", now), "T+00:05:12");
assert.strictEqual(M.barText(L({}), now, true), "Falcon 9 · T-02:14:07");
assert.strictEqual(M.barText(L({}), now, false), "T-02:14:07");
assert.strictEqual(M.barText(null, now, true), "—");
assert.strictEqual(M.ICON, "\u{F14DE}");
assert.ok(!/🚀|●|LIVE/.test(M.barText(L({ status_id: 6, status_class: "flight" }), now, true, "MECO")));
assert.ok(M.displayCountdown(L({ net: "2026-10-20T00:00:00Z", net_precision: "DAY" }), now).startsWith("NET "));
// coarse NETs: UTC calendar (matches the daemon), never the previous year west of UTC
[["Y", "NET 2027"], ["M", "NET Jan 2027"], ["Q1", "NET Q1 2027"], ["H1", "NET H1 2027"]].forEach(function (c) {
  assert.strictEqual(M.displayCountdown(L({ net: "2027-01-01T00:00:00Z", net_precision: c[0] }), now), c[1]);
});
const cache = { starred: ["SpaceX"], launches: [
  L({ id: "done", status_id: 3, net: "2026-10-13T17:00:00Z" }),
  L({ id: "rl", provider: "Rocket Lab", provider_abbrev: "RL", net: "2026-10-13T19:00:00Z" }),
  L({ id: "sx", net: "2026-10-13T21:00:00Z" }),
  L({ id: "old", net: "2026-10-13T16:00:00Z" }),
] };
assert.deepStrictEqual(M.upcoming(cache, now).map(l => l.id), ["rl", "sx"]);
assert.deepStrictEqual(M.upcoming(cache, now, { providerFilter: "spacex" }).map(l => l.id), ["sx"]);
assert.deepStrictEqual(M.upcoming(cache, now, { providerFilter: "RL, ula" }).map(l => l.id), ["rl"]);
assert.deepStrictEqual(M.upcoming(cache, now, { starredOnly: true }).map(l => l.id), ["sx"]);
assert.ok(M.isLive(L({ net: "2026-10-13T18:30:00Z" }), now, 60));
assert.ok(!M.isLive(L({ net: "2026-10-13T20:30:00Z" }), now, 60));
assert.ok(M.isLive(L({ status_id: 6, net: "2026-10-13T17:00:00Z" }), now, 60));
assert.ok(M.hidden(L({ net: "2026-10-30T00:00:00Z" }), now, 7));
assert.ok(!M.hidden(L({ net: "2026-10-30T00:00:00Z" }), now, 0));
assert.deepStrictEqual(M.parseCache("garbage").launches, []);
// real cache produced by the daemon, if present
const real = process.env.LIFTOFF_CACHE;
if (real && fs.existsSync(real)) {
  const c = M.parseCache(fs.readFileSync(real, "utf8"));
  const up = M.upcoming(c, Date.now());
  console.log("live cache:", M.barText(up[0], Date.now(), true), "| next5:", up.slice(0, 5).map(M.shortRocket).join(", "));
}
// icon state: normal / live (accent, pulsing) / bad (red, static)
assert.strictEqual(M.iconState(L({ status_class: "go" }), now), "normal");
assert.strictEqual(M.iconState(L({ status_class: "tbd", status_id: 2 }), now), "normal");
assert.strictEqual(M.iconState(L({ status_class: "tbc", status_id: 8 }), now), "normal");
assert.strictEqual(M.iconState(L({ status_class: "success", status_id: 3, net: "2026-10-13T17:00:00Z" }), now), "normal");
assert.strictEqual(M.iconState(L({ status_class: "flight", status_id: 6 }), now), "live");
assert.strictEqual(M.iconState(L({ webcast_live: true }), now), "live");
assert.strictEqual(M.iconState(L({}), now, "Engine chill"), "live");
assert.strictEqual(M.iconState(L({ net: "2026-10-13T17:58:00Z" }), now), "live");       // past NET, not finished
assert.strictEqual(M.iconState(L({ status_class: "hold", status_id: 5 }), now), "bad");
assert.strictEqual(M.iconState(L({ status_class: "hold", status_id: 5, webcast_live: true }), now), "live"); // live wins over hold
assert.strictEqual(M.iconState(L({ status_class: "failure", status_id: 4, webcast_live: true }), now), "bad"); // failure wins
assert.strictEqual(M.iconState(L({ status_class: "failure", status_id: 7, net: "2026-10-13T17:58:00Z" }), now), "bad");
assert.strictEqual(M.iconState(null, now), "normal");
// next milestone: flight plan wins over LL2 timeline
const ms = L({ id: "m", net: "2026-10-13T18:05:00Z", milestones: [{ offset_secs: -60, label: "Startup" }] });
const plans = { flight_plans: { m: { events: [{ offset_secs: -420, label: "Engine chill" }, { offset_secs: 0, label: "Liftoff" }, { offset_secs: 146, label: "MECO" }] } } };
assert.strictEqual(M.nextMilestone(plans, ms, now).label, "Liftoff");          // now = T-05:00
assert.strictEqual(M.nextMilestone(plans, ms, now).at, "T+00:00");
assert.strictEqual(M.nextMilestone({}, ms, now).label, "Startup");
assert.strictEqual(M.nextMilestone(plans, ms, now + 6 * 60000).label, "MECO");  // T+01:00
assert.strictEqual(M.nextMilestone(plans, ms, now + 10 * 60000), null);         // T+05:00, past MECO
// live stage (bar + popup)
const lv = L({ id: "m", net: "2026-10-13T17:57:29Z", status_id: 6, status_class: "flight" }); // now = T+02:31
const st = M.liveStage(plans, lv, now);
assert.strictEqual(st.current.label, "MECO");
assert.strictEqual(st.next, null);
assert.strictEqual(st.source, "spacex.com");
assert.strictEqual(M.barText(lv, now, true, st.current.label), "Falcon 9 · MECO · T+00:02:31");
const pre = M.liveStage(plans, L({ id: "m", net: "2026-10-13T18:05:00Z" }), now);   // T-05:00
assert.strictEqual(pre.current.label, "Engine chill");
assert.strictEqual(pre.next.label, "Liftoff");
assert.strictEqual(pre.next.in, 300);
assert.strictEqual(M.liveStage(plans, L({ id: "m", net: "2026-10-13T20:00:00Z" }), now), null); // T-2h: not live
assert.strictEqual(M.liveStage({}, L({ id: "none" }), now), null);                             // no events
assert.strictEqual(M.liveStage({}, L({ id: "z", net: "2026-10-13T18:05:00Z", net_precision: "DAY", milestones: [{ offset_secs: 0, label: "Liftoff" }] }), now), null);
assert.strictEqual(M.shortSecs(305), "5m05s");
assert.ok(M.barText(L({}), now, true, "").indexOf(" ·  · ") === -1);
// T-0 nudge + clock skew shared with the TUI
const base = { sync: { clock_offset_ms: 0 }, launches: [L({ id: "n", net: "2026-10-13T18:00:00Z" }), L({ id: "o", net: "2026-10-13T19:00:00Z" })] };
const nd = { launch_id: "n", t0: "2026-10-13T18:00:03Z", base_net: "2026-10-13T18:00:00Z" };
assert.strictEqual(Date.parse(M.applyClock(base, nd).launches[0].net), Date.parse("2026-10-13T18:00:03Z"));
assert.strictEqual(M.applyClock(base, nd).launches[1], base.launches[1]);
const moved = { launches: [L({ id: "n", net: "2026-10-13T18:30:00Z" })] };
assert.strictEqual(M.applyClock(moved, nd).launches[0].net, "2026-10-13T18:30:00Z"); // recycle -> ignored
const skew = { sync: { clock_offset_ms: 2000 }, launches: [L({ id: "o", net: "2026-10-13T18:00:00Z" })] };
assert.strictEqual(M.countdown(M.applyClock(skew, {}).launches[0].net, now - 10000), "T-00:00:08"); // clock 2 s slow
assert.strictEqual(M.applyClock(base, {}), base);
assert.deepStrictEqual(M.parseNudge("junk"), {});
// section-dependent text
assert.strictEqual(M.showText("auto", "center", false), true);
assert.strictEqual(M.showText("auto", "left", false), false);
assert.strictEqual(M.showText("auto", "right", false), false);
assert.strictEqual(M.showText("auto", "tray", false), false);
assert.strictEqual(M.showText("auto", "", false), true);        // unknown -> text
assert.strictEqual(M.showText(undefined, "center", false), true);
assert.strictEqual(M.showText("auto", "center", true), false);  // vertical bar
assert.strictEqual(M.showText("always", "left", true), true);
assert.strictEqual(M.showText("never", "center", false), false);
assert.strictEqual(M.showText("Always", "right", false), true);
// test flight marker (display only, heartbeat)
const tl = L({ id: "liftoff-test-flight", net: "2026-10-13T17:57:29Z", status_id: 6, status_class: "flight", rocket: "Falcon 9" });
const mk = M.parseTest(JSON.stringify({ test: true, heartbeat_ms: now - 1000, launch: tl, flight_plan: plans.flight_plans.m }));
assert.ok(M.testActive(mk, now));
assert.ok(!M.testActive(Object.assign({}, mk, { heartbeat_ms: now - 6000 }), now));   // stale -> crashed TUI ignored
assert.strictEqual(M.parseTest("junk"), null);
assert.strictEqual(M.parseTest(JSON.stringify({ test: false, stopped_ms: now })), null);   // stop record -> bar normal at once
const realC = { launches: [L({ id: "r", net: "2026-10-13T19:00:00Z" })], flight_plans: {} };
const mc = M.withTest(realC, mk, now);
assert.strictEqual(mc.launches[0].id, "liftoff-test-flight");
assert.strictEqual(realC.launches.length, 1);                                           // real cache untouched
assert.strictEqual(M.withTest(realC, Object.assign({}, mk, { heartbeat_ms: now - 60000 }), now), realC);
assert.strictEqual(M.upcomingWithTest(mc, now, {})[0].id, "liftoff-test-flight");
const tst = M.liveStage(mc, mc.launches[0], now);
assert.strictEqual(tst.current.label, "MECO");
assert.strictEqual(M.barText(mc.launches[0], now, true, tst.current.label), "Test Flight · MECO · T+00:02:31");
assert.strictEqual(M.iconState(mc.launches[0], now, "MECO"), "live");
// countdown matches the TUI rule (same table as schedule.rs countdown_rule_matches_bar)
{
  const n = Date.parse("2026-10-13T12:00:00Z");
  for (const [off, want] of [[8534700, "T-02:22:15"], [8534000, "T-02:22:14"], [1999, "T-00:00:02"], [1000, "T-00:00:01"],
    [999, "T-00:00:01"], [1, "T-00:00:01"], [0, "T-00:00:00"], [-1, "T-00:00:00"], [-999, "T-00:00:00"],
    [-1000, "T+00:00:01"], [-1001, "T+00:00:01"], [-312400, "T+00:05:12"]]) {
    assert.strictEqual(M.countdown(new Date(n + off).toISOString(), n), want, "net = now + " + off + " ms");
  }
  assert.strictEqual(M.msToNextSecond(n + 250), 755);
  assert.strictEqual(M.msToNextSecond(n), 1005);
  assert.strictEqual(M.msToNextSecond(n, { sync: { clock_offset_ms: 2947 } }), 58);   // corrected clock at .947
  // bar (applyClock) and TUI (now + offset) agree with a skew applied
  const sk = { launches: [L({ id: "s", net: new Date(n + 60000).toISOString() })], sync: { clock_offset_ms: 2947 } };
  assert.strictEqual(M.countdown(M.applyClock(sk, null).launches[0].net, n), "T-00:00:58");

}
console.log("model.test.js: all assertions passed");

// Coarse NETs are calendar labels: identical text in every TZ (run under Phoenix, UTC, Tokyo).
// [code, net, short, long, column] — nets are LL2's real encoding (00:00Z) plus 23:59Z.
[["DAY", "2026-11-07T00:00:00Z", "Nov 7", "Sat Nov 7", "Nov 7"], ["DAY", "2026-11-07T23:59:00Z", "Nov 7", "Sat Nov 7", "Nov 7"],
 ["M", "2026-12-31T00:00:00Z", "Dec 2026", "Dec 2026", "Dec 2026"], ["Q4", "2026-12-31T00:00:00Z", "Q4 2026", "Q4 2026", "Q4 2026"],
 ["Q1", "2027-01-01T00:00:00Z", "Q1 2027", "Q1 2027", "Q1 2027"], ["H1", "2027-06-30T00:00:00Z", "H1 2027", "H1 2027", "H1 2027"],
 ["Y", "2026-12-31T00:00:00Z", "2026", "2026", "2026"], ["Y", "2027-01-01T00:00:00Z", "2027", "2027", "2027"],
 ["FY", "2027-06-30T00:00:00Z", "~2027 (fiscal year)", "~2027 (fiscal year)", "~2027"], ["DEC", "2029-12-31T00:00:00Z", "~2020s", "~2020s", "~2020s"],
 ["AM", "2026-11-03T00:00:00Z", "~Nov 3 (morning, site time)", "~Nov 3 (morning, site time)", "~Nov 3"],
 ["PM", "2026-11-03T23:59:00Z", "~Nov 3 (afternoon, site time)", "~Nov 3 (afternoon, site time)", "~Nov 3"],
 ["WK", "2026-11-09T00:00:00Z", "~Nov 9 (week)", "~Nov 9 (week)", "~Nov 9"],
 ["W", "2026-11-09T00:00:00Z", "~Nov 9 (week)", "~Nov 9 (week)", "~Nov 9"], ["Q", "2026-12-31T00:00:00Z", "Q4 2026", "Q4 2026", "Q4 2026"],
 ["ZZZ", "2026-11-07T00:00:00Z", "~Nov 7", "~Nov 7", "~Nov 7"]].forEach(function (c) {
  const l = L({ net: c[1], net_precision: c[0] });
  assert.strictEqual(M.netText(l, "short"), c[2], c[0] + " short");
  assert.strictEqual(M.netText(l, "long"), c[3], c[0] + " long");
  assert.strictEqual(M.netText(l, "column"), c[4], c[0] + " column");
  assert.strictEqual(M.displayCountdown(l, now), "NET " + c[3], c[0]);
  // no countdown toward the placeholder even within the hour
  assert.strictEqual(M.displayCountdown(l, Date.parse(c[1]) - 600000), "NET " + c[3], c[0] + " near");
});
assert.ok(M.netText(L({ net_precision: "HR" }), "clock").endsWith("±1h"));
// 0.1.5: bounded reads and home expansion for the binary path
assert.strictEqual(M.MAX_FILE_BYTES, 2 * 1024 * 1024);
assert.ok(!M.tooBig(150000) && M.tooBig(M.MAX_FILE_BYTES + 1) && M.tooBig(NaN));
const huge = '{"launches":[' + " ".repeat(M.MAX_FILE_BYTES) + "]}";
assert.match(M.parseCache(huge).error, /too large/);
assert.deepStrictEqual(M.parseCache(huge).launches, []);
assert.strictEqual(M.parseNudge(huge).launches, undefined);
assert.strictEqual(M.parseTest(huge), null);
assert.strictEqual(M.expandHome("$HOME/.local/bin/liftoff", "/home/u"), "/home/u/.local/bin/liftoff");
assert.strictEqual(M.expandHome("${HOME}/bin/x", "/home/u"), "/home/u/bin/x");
assert.strictEqual(M.expandHome("~/x y/liftoff", "/home/u"), "/home/u/x y/liftoff");
assert.strictEqual(M.expandHome("/opt/liftoff; rm -rf ~", "/home/u"), "/opt/liftoff; rm -rf ~");
console.log("model.test.js: bounded-read + expandHome assertions passed");

// B1: one expiry rule, cases shared with src/model.rs (tests/fixtures/expiry_cases.json)
const EX = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "..", "tests", "fixtures", "expiry_cases.json"), "utf8"));
const wrong = EX.cases.filter((c) => {
  const l = L({ net: c.net, net_precision: c.p, status_id: c.status_id || 1 });
  return (M.upcoming({ launches: [l] }, Date.parse(c.now)).length === 0) !== c.expired;
});
assert.deepStrictEqual(wrong, [], "expiry rule disagrees with tests/fixtures/expiry_cases.json:\n" + wrong.map((c) => JSON.stringify(c)).join("\n"));
// a calendar label has no T-0: never "live" / pulsing at 00:00 UTC
const dayL = L({ net: "2026-10-14T00:00:00Z", net_precision: "DAY" });
assert.strictEqual(M.isLive(dayL, Date.parse("2026-10-13T23:30:00Z"), 60), false);
assert.strictEqual(M.iconState(dayL, Date.parse("2026-10-14T01:00:00Z")), "normal");
console.log("model.test.js: expiry assertions passed (" + EX.cases.length + " shared cases)");
