// Static checks on BarWidget.qml (no QML runtime in CI).
const assert = require("assert");
const q = require("fs").readFileSync(require("path").join(__dirname, "..", "BarWidget.qml"), "utf8");
const block = (id) => { const i = q.indexOf("id: " + id); assert.ok(i > 0, "missing " + id); return q.slice(q.lastIndexOf("Timer {", i), q.indexOf("\n  }", i)); };
// v0.1.1: after install.sh the cache appears later; poll it while "set up OmaLiftoff" is shown
const sp = block("setupPoll");
assert.match(sp, /running:\s*root\.needsSetup/);
assert.match(sp, /repeat:\s*true/);
assert.match(sp, /cacheFile\.reload\(\)/);
// clock re-arms on whole-second boundaries (matches the TUI countdown)
const ct = block("clockTick");
assert.match(ct, /Model\.msToNextSecond/);
assert.match(ct, /restart\(\)/);
console.log("widget.test.js: all assertions passed");
// 0.1.5: no shell for refresh; the binary path is one argv element; cache size checked before parsing
assert.ok(!/bar\.run\(liftoffBinary/.test(q), "liftoffBinary passed through a shell unquoted");
assert.match(q, /Quickshell\.execDetached\(\[liftoffPath, "refresh"\]\)/);
assert.match(q, /Util\.shellQuote\(Util\.shellQuote\(liftoffPath\) \+ " tui"\)/);
assert.match(q, /Model\.tooBig\(d \? d\.byteLength : 0\)/);
console.log("widget.test.js: argv/bounded-read assertions passed");
